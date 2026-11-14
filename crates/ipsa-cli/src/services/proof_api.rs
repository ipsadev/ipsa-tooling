use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::{config::get, logger, probe, services::cosmos::config::CosmosConfig};

const PID_FILE: &str = "proof-api.pid";

const LOG_FILE: &str = "proof-api.log";

const START_TIMEOUT: Duration = Duration::from_secs(180);

const STOP_TIMEOUT: Duration = Duration::from_secs(30);

const INLINE_CONFIG_VARIABLE: &str = "PROOF_API_CONFIG_JSON";

const POLL: Duration = Duration::from_millis(500);

const PROCESS_NAME: &str = "stellar-proof-api";

#[derive(clap::Subcommand)]
pub enum ProofApiCmd {
    #[command(about = "Start stellar-proof-api from PROOF_API_DIR unless it is already listening")]
    Start,
    #[command(about = "Stop the stellar-proof-api listening on PROOF_API_URL")]
    Stop,
    #[command(about = "Stop, then start, stellar-proof-api — it rereads its config")]
    Restart,
    #[command(about = "Report whether stellar-proof-api is listening on PROOF_API_URL")]
    Status,
}

pub struct ProofApiConfig {
    directory: String,
    binary: String,
    config: String,
    env_file: String,
    host: String,
    port: u16,
}

impl ProofApiConfig {
    pub fn from_env() -> Result<Self> {
        let url = get("PROOF_API_URL", "http://127.0.0.1:3100");
        let parsed = reqwest::Url::parse(&url)
            .with_context(|| format!("PROOF_API_URL ({url}) is not a URL"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("PROOF_API_URL ({url}) names no host"))?
            .to_string();
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| anyhow!("PROOF_API_URL ({url}) names no port"))?;

        Ok(Self {
            directory: get("PROOF_API_DIR", ""),
            binary: get("PROOF_API_BINARY", "target/release/stellar-proof-api"),
            config: get(
                "PROOF_API_CONFIG",
                "programs/stellar-proof-api/testnet.json",
            ),
            env_file: get("PROOF_API_ENV_FILE", ".env"),
            host,
            port,
        })
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn directory(&self) -> Result<PathBuf> {
        if self.directory.is_empty() {
            bail!(
                "PROOF_API_DIR is not set — add the stellar-proof-api checkout (ibc-contracts) to {}",
                crate::config::tooling_env().display()
            );
        }

        let directory = PathBuf::from(&self.directory);

        if !directory.is_dir() {
            bail!("PROOF_API_DIR ({}) is not a directory", directory.display());
        }

        Ok(directory)
    }
}

pub fn run(root: &Path, cmd: ProofApiCmd) -> Result<()> {
    let cfg = ProofApiConfig::from_env()?;

    match cmd {
        ProofApiCmd::Start => start(root, &cfg),
        ProofApiCmd::Stop => stop(root, &cfg),
        ProofApiCmd::Restart => {
            stop(root, &cfg)?;
            start(root, &cfg)
        }
        ProofApiCmd::Status => {
            status(root, &cfg);

            Ok(())
        }
    }
}

pub fn listening(cfg: &ProofApiConfig) -> bool {
    probe::tcp_ok(&cfg.address())
}

pub fn start(root: &Path, cfg: &ProofApiConfig) -> Result<()> {
    logger::banner("proof-api start");

    if listening(cfg) {
        logger::ok(&format!("already listening on {}", cfg.address()));

        return Ok(());
    }

    let directory = cfg.directory()?;
    let binary = directory.join(&cfg.binary);

    if !binary.is_file() {
        bail!(
            "{} does not exist — build it in {}: cargo build --release -p stellar-proof-api",
            binary.display(),
            directory.display()
        );
    }

    let config = directory.join(&cfg.config);

    if !config.is_file() {
        bail!("{} does not exist (PROOF_API_CONFIG)", config.display());
    }

    let env_file = directory.join(&cfg.env_file);
    let variables = load_env_file(&env_file)?;

    let cosmos_chain = CosmosConfig::from_env().chain_id.as_str().to_string();
    let stellar_chain = get("STELLAR_CHAIN_ID", "stellar-testnet");
    let inline = forwarding_to(&fs::read_to_string(&config)?, &stellar_chain, &cosmos_chain)
        .with_context(|| format!("reading {}", config.display()))?;

    let log_path = root.join(LOG_FILE);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let errors = log.try_clone()?;

    logger::step(&format!(
        "{} start --config {} (in {}, {} variables from {}, forwarding {stellar_chain} ↔ {cosmos_chain})",
        cfg.binary,
        cfg.config,
        directory.display(),
        variables.len(),
        env_file.display()
    ));

    let mut child = Command::new(&binary)
        .args(["start", "--config", &cfg.config])
        .current_dir(&directory)
        .envs(variables)
        .env(INLINE_CONFIG_VARIABLE, inline)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(errors)
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting {}", binary.display()))?;

    let pid = child.id();

    fs::write(root.join(PID_FILE), format!("{pid}\n"))
        .with_context(|| format!("writing {}", root.join(PID_FILE).display()))?;

    let deadline = Instant::now() + START_TIMEOUT;

    while Instant::now() < deadline {
        if let Some(exit) = child.try_wait()? {
            let _ = fs::remove_file(root.join(PID_FILE));

            bail!(
                "stellar-proof-api exited during startup ({exit}) — see {}",
                log_path.display()
            );
        }

        if listening(cfg) {
            logger::ok(&format!(
                "listening on {} (pid {pid}, log {})",
                cfg.address(),
                log_path.display()
            ));

            return Ok(());
        }

        thread::sleep(POLL);
    }

    bail!(
        "stellar-proof-api (pid {pid}) is not listening on {} after {}s — see {}",
        cfg.address(),
        START_TIMEOUT.as_secs(),
        log_path.display()
    )
}

pub fn stop(root: &Path, cfg: &ProofApiConfig) -> Result<()> {
    logger::banner("proof-api stop");

    let mut pids = listening_pids(cfg.port);

    if let Some(recorded) = recorded_pid(root) {
        if alive(recorded) && !pids.contains(&recorded) {
            pids.push(recorded);
        }
    }

    if pids.is_empty() {
        logger::ok(&format!("nothing is listening on {}", cfg.address()));
        let _ = fs::remove_file(root.join(PID_FILE));

        return Ok(());
    }

    for &pid in &pids {
        let command = command_of(pid);

        if !command.contains(PROCESS_NAME) {
            bail!(
                "pid {pid} on {} is not stellar-proof-api ({command}) — refusing to stop it",
                cfg.address()
            );
        }
    }

    for &pid in &pids {
        logger::step(&format!("stopping pid {pid}"));
        signal(pid, "TERM");
    }

    let deadline = Instant::now() + STOP_TIMEOUT;

    while Instant::now() < deadline {
        if pids.iter().all(|&pid| !alive(pid)) && !listening(cfg) {
            let _ = fs::remove_file(root.join(PID_FILE));
            logger::ok("stopped");

            return Ok(());
        }

        thread::sleep(POLL);
    }

    for &pid in pids.iter().filter(|&&pid| alive(pid)) {
        logger::warn(&format!(
            "pid {pid} ignored SIGTERM for {}s — sending SIGKILL",
            STOP_TIMEOUT.as_secs()
        ));
        signal(pid, "KILL");
    }

    let _ = fs::remove_file(root.join(PID_FILE));
    logger::ok("stopped");

    Ok(())
}

fn status(root: &Path, cfg: &ProofApiConfig) {
    logger::banner("proof-api status");

    let pids = listening_pids(cfg.port);

    if listening(cfg) {
        let owner = pids
            .first()
            .map(|pid| format!("pid {pid}"))
            .unwrap_or_else(|| "pid unknown".to_string());

        logger::ok(&format!("listening on {} ({owner})", cfg.address()));
    } else {
        logger::fail(&format!("nothing is listening on {}", cfg.address()));
    }

    logger::detail(&format!("log: {}", root.join(LOG_FILE).display()));
}

pub fn forwarding_to(config: &str, stellar_chain: &str, cosmos_chain: &str) -> Result<String> {
    let mut config: Value = serde_json::from_str(config)?;

    let modules = config
        .get_mut("modules")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("the proof api config has no modules array"))?;

    for module in modules {
        if module.get("name").and_then(Value::as_str) != Some("forward") {
            continue;
        }

        for side in ["src_chain", "dst_chain"] {
            let is_stellar = module.get(side).and_then(Value::as_str) == Some(stellar_chain);

            if !is_stellar {
                module[side] = Value::String(cosmos_chain.to_string());
            }
        }
    }

    Ok(serde_json::to_string(&config)?)
}

fn load_env_file(path: &Path) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        logger::warn(&format!(
            "{} does not exist — starting without it (the network prover needs NETWORK_PRIVATE_KEY)",
            path.display()
        ));

        return Ok(Vec::new());
    }

    dotenvy::from_path_iter(path)
        .with_context(|| format!("reading {}", path.display()))?
        .map(|item| item.with_context(|| format!("parsing {}", path.display())))
        .collect()
}

fn recorded_pid(root: &Path) -> Option<u32> {
    fs::read_to_string(root.join(PID_FILE))
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

fn listening_pids(port: u16) -> Vec<u32> {
    let output = Command::new("lsof")
        .args(["-nP", "-t", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn command_of(pid: u32) -> String {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn signal(pid: u32, name: &str) {
    let _ = Command::new("kill")
        .args([&format!("-{name}"), &pid.to_string()])
        .status();
}
