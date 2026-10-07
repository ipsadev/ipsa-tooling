use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde_json::Value;

use crate::{config::Config, logger, shared::Chain, tx::evm::config::EvmCorridorConfig};

mod budget;
mod config;

pub use config::RelayConfig;

#[derive(clap::Args)]
pub struct RelayArgs {
    #[arg(
        long,
        value_name = "HASH",
        help = "Hash of the source transaction that emitted the SendPacket"
    )]
    pub tx: String,
    #[arg(
        long,
        help = "The tx is on Stellar — relay Stellar → Cosmos (inferred from the hash when omitted)"
    )]
    pub stellar: bool,
    #[arg(
        long,
        help = "The tx is on Cosmos — relay Cosmos → Stellar (inferred from the hash when omitted)"
    )]
    pub cosmos: bool,
    #[arg(
        long,
        help = "The tx is on Stellar and belongs to the Stellar ↔ EVM corridor — relay Stellar → EVM (an 0x hash is always EVM → Stellar)"
    )]
    pub evm: bool,
    #[arg(
        long,
        help = "Stellar ↔ EVM only: prove and submit through the proof API from here instead of handing the tx to the relayer"
    )]
    pub direct: bool,
    #[arg(
        long,
        value_name = "ID",
        help = "Source chain id to send, overriding STELLAR_CHAIN_ID / COSMOS_CHAIN_ID"
    )]
    pub chain_id: Option<String>,
    #[arg(
        long,
        help = "Only report the packet status; do not submit the tx for relaying"
    )]
    pub status: bool,
    #[arg(long, help = "Poll the status until every packet completes or fails")]
    pub wait: bool,
    #[arg(
        long,
        help = "With --wait, exit non-zero if the packets have not settled by the timeout"
    )]
    pub strict: bool,
    #[arg(
        long,
        help = "Give up waiting after this many seconds (--wait); omit to derive it from the watcher"
    )]
    pub timeout_secs: Option<u64>,
    #[arg(
        long,
        default_value_t = 5,
        help = "Seconds between status polls (--wait)"
    )]
    pub interval_secs: u64,
}

struct Source {
    chain: Chain,
    destination: Chain,
    id: String,
    inferred: bool,
}

impl Source {
    fn direction(&self) -> String {
        format!("{} → {}", self.chain.as_str(), self.destination.as_str())
    }
}

struct Opts {
    status: bool,
    wait: bool,
    strict: bool,
    timeout_secs: Option<u64>,
    interval_secs: u64,
}

impl From<&RelayArgs> for Opts {
    fn from(args: &RelayArgs) -> Self {
        Self {
            status: args.status,
            wait: args.wait,
            strict: args.strict,
            timeout_secs: args.timeout_secs,
            interval_secs: args.interval_secs,
        }
    }
}

pub async fn run(cfg: &Config, http: &reqwest::Client, args: RelayArgs) -> Result<()> {
    let tx = args.tx.trim().to_string();

    if tx.is_empty() {
        bail!("--tx is required — pass the hash of the transfer transaction");
    }

    if args.direct {
        return direct(cfg, &tx, args.evm).await;
    }

    let source = source_chain(cfg, &args, &tx)?;

    relay(cfg, http, &tx, &source, &Opts::from(&args)).await
}

async fn direct(cfg: &Config, tx: &str, evm: bool) -> Result<()> {
    if tx.starts_with("0x") {
        let root = crate::repo::find_root()?;

        return crate::tx::evm::relay::evm_to_stellar(cfg, &root, tx)
            .await
            .map(|_| ());
    }

    if evm {
        return crate::tx::evm::relay::stellar_to_evm(cfg, tx)
            .await
            .map(|_| ());
    }

    bail!("--direct applies to the Stellar ↔ EVM corridor only — pass --evm or an 0x hash");
}

pub async fn after_transfer(
    cfg: &Config,
    http: &reqwest::Client,
    route: (Chain, Chain),
    tx: &str,
    wait: bool,
    strict: bool,
    timeout_secs: Option<u64>,
) -> Result<()> {
    let (chain, destination) = route;

    let source = Source {
        id: chain_id(cfg, chain, destination),
        chain,
        destination,
        inferred: false,
    };

    let opts = Opts {
        status: false,
        wait,
        strict,
        timeout_secs,
        interval_secs: 5,
    };

    relay(cfg, http, tx, &source, &opts).await
}

async fn relay(
    cfg: &Config,
    http: &reqwest::Client,
    tx: &str,
    source: &Source,
    opts: &Opts,
) -> Result<()> {
    let rc = RelayConfig::from(cfg);

    logger::banner(&format!("tx relay ({})", source.direction()));

    if source.inferred {
        logger::detail(&format!(
            "read {} as a {} transaction — pass --stellar/--cosmos to be explicit",
            tx,
            source.chain.as_str()
        ));
    }

    if !opts.status {
        submit(&rc, http, tx, source).await?;
    }

    if opts.wait {
        let budget = budget::derive(
            &rc,
            http,
            source.chain,
            source.destination,
            opts.timeout_secs,
        )
        .await;
        budget::announce(&budget);

        return wait_for_packets(&rc, http, tx, source, opts, &budget).await;
    }

    report(&rc, http, tx, source).await?;

    if !opts.status {
        logger::hint("follow the relay: docker compose logs -f relayer");
        logger::hint(&format!(
            "or poll it: ipsa tx relay --tx {tx} --status --wait"
        ));
    }

    Ok(())
}

// A cosmos tx broadcast in sync mode returns once it is in the mempool, so the
// relayer can be asked for it before any block contains it. That reads as a
// missing tx, not a pending one, which is why this waits rather than failing.
const INCLUSION_WAIT: Duration = Duration::from_secs(90);

const INCLUSION_POLL: Duration = Duration::from_secs(3);

async fn submit(rc: &RelayConfig, http: &reqwest::Client, tx: &str, source: &Source) -> Result<()> {
    logger::step(&format!("relay {tx} (chain {})", source.id));

    let body = serde_json::json!({ "tx_hash": tx, "chain_id": source.id });
    let started = Instant::now();
    let mut waited = false;

    loop {
        let response = http
            .post(rc.relay_url())
            .json(&body)
            .send()
            .await
            .map_err(|error| unreachable_api(rc, error))?;

        let status = response.status();
        let payload: Value = response.json().await.unwrap_or(Value::Null);

        if status.is_success() {
            logger::ok("accepted — the relayer will build, sign and submit the packet messages");

            return Ok(());
        }

        if status != reqwest::StatusCode::NOT_FOUND || started.elapsed() >= INCLUSION_WAIT {
            return Err(relay_error(rc, status, &payload));
        }

        if !waited {
            logger::detail("not in a block yet — waiting for the tx to be included");
            waited = true;
        }

        tokio::time::sleep(INCLUSION_POLL).await;
    }
}

async fn report(rc: &RelayConfig, http: &reqwest::Client, tx: &str, source: &Source) -> Result<()> {
    match packets(rc, http, tx, source).await? {
        Some(packets) => {
            print_packets(&packets);

            Ok(())
        }
        None => {
            logger::detail("the relayer is not tracking this tx yet");

            Ok(())
        }
    }
}

async fn wait_for_packets(
    rc: &RelayConfig,
    http: &reqwest::Client,
    tx: &str,
    source: &Source,
    opts: &Opts,
    budget: &budget::Budget,
) -> Result<()> {
    let deadline = Duration::from_secs(budget.seconds);
    let interval = Duration::from_secs(opts.interval_secs.max(1));
    let started = Instant::now();

    logger::step(&format!(
        "waiting up to {}s for the packets to settle",
        budget.seconds
    ));

    let mut last = String::new();

    loop {
        if let Some(packets) = packets(rc, http, tx, source).await? {
            let rendered = render(&packets);

            if rendered != last {
                print_packets(&packets);
                last = rendered;
            }

            match outcome(&packets) {
                Outcome::Failed { failed, total } => {
                    bail!("{failed} of {total} packets failed to relay")
                }
                Outcome::Relayed { total } => {
                    logger::ok(&format!("{total} packet(s) relayed"));

                    return Ok(());
                }
                Outcome::Pending => {}
            }
        }

        if started.elapsed() >= deadline {
            if opts.strict {
                logger::fail(&format!(
                    "packets did not settle within {}s ({})",
                    budget.seconds, budget.reason
                ));

                bail!(
                    "{}: packets did not settle within {}s",
                    source.direction(),
                    budget.seconds
                );
            }

            logger::warn(&format!(
                "still pending after {}s — the relayer keeps retrying in the background",
                budget.seconds
            ));

            return Ok(());
        }

        tokio::time::sleep(interval).await;
    }
}

async fn packets(
    rc: &RelayConfig,
    http: &reqwest::Client,
    tx: &str,
    source: &Source,
) -> Result<Option<Vec<Packet>>> {
    let response = http
        .get(rc.status_url())
        .query(&[("tx_hash", tx), ("chain_id", source.id.as_str())])
        .send()
        .await
        .map_err(|error| unreachable_api(rc, error))?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    let payload: Value = response.json().await.unwrap_or(Value::Null);

    if !status.is_success() {
        return Err(relay_error(rc, status, &payload));
    }

    let packets = payload
        .get("packets")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().map(Packet::from).collect())
        .unwrap_or_default();

    Ok(Some(packets))
}

enum Outcome {
    Pending,
    Relayed { total: usize },
    Failed { failed: usize, total: usize },
}

fn outcome(packets: &[Packet]) -> Outcome {
    if packets.is_empty() || !packets.iter().all(Packet::settled) {
        return Outcome::Pending;
    }

    let failed = packets.iter().filter(|p| p.state == "FAILED").count();

    if failed > 0 {
        return Outcome::Failed {
            failed,
            total: packets.len(),
        };
    }

    Outcome::Relayed {
        total: packets.len(),
    }
}

struct Packet {
    state: String,
    sequence: u64,
    source_client_id: String,
    recv: Option<String>,
    ack: Option<String>,
    timeout: Option<String>,
}

impl Packet {
    fn settled(&self) -> bool {
        matches!(self.state.as_str(), "COMPLETE" | "FAILED")
    }
}

impl From<&Value> for Packet {
    fn from(value: &Value) -> Self {
        let hash = |field: &str| {
            value
                .get(field)
                .and_then(|tx| tx.get("tx_hash"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };

        Self {
            state: value
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string(),
            sequence: value.get("sequence").and_then(Value::as_u64).unwrap_or(0),
            source_client_id: value
                .get("source_client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string(),
            recv: hash("recv_tx"),
            ack: hash("ack_tx"),
            timeout: hash("timeout_tx"),
        }
    }
}

fn print_packets(packets: &[Packet]) {
    if packets.is_empty() {
        logger::detail("no packets found in that transaction");

        return;
    }

    for packet in packets {
        let line = format!(
            "seq {} (client {}): {}",
            packet.sequence, packet.source_client_id, packet.state
        );

        match packet.state.as_str() {
            "COMPLETE" => logger::ok(&line),
            "FAILED" => logger::fail(&line),
            _ => logger::detail(&line),
        }

        for (label, hash) in [
            ("recv", &packet.recv),
            ("ack", &packet.ack),
            ("timeout", &packet.timeout),
        ] {
            if let Some(hash) = hash {
                logger::detail(&format!("  {label}: {hash}"));
            }
        }
    }
}

fn render(packets: &[Packet]) -> String {
    packets
        .iter()
        .map(|p| {
            format!(
                "{}:{}:{}:{}:{}",
                p.sequence,
                p.state,
                p.recv.as_deref().unwrap_or(""),
                p.ack.as_deref().unwrap_or(""),
                p.timeout.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn source_chain(cfg: &Config, args: &RelayArgs, tx: &str) -> Result<Source> {
    let (chain, destination, inferred) = if tx.starts_with("0x") {
        (Chain::Evm, Chain::Stellar, false)
    } else if args.evm {
        (Chain::Stellar, Chain::Evm, false)
    } else {
        let chain = match (args.cosmos, args.stellar) {
            (true, true) => bail!("pass either --cosmos or --stellar, not both"),
            (true, false) => Some(Chain::Cosmos),
            (false, true) => Some(Chain::Stellar),
            _ => infer_chain(tx),
        };

        let Some(chain) = chain else {
            bail!(
                "could not tell which chain {tx} is on — pass --stellar (relay to cosmos) or --cosmos (relay to stellar)"
            );
        };

        let destination = match chain {
            Chain::Cosmos => Chain::Stellar,
            _ => Chain::Cosmos,
        };

        (chain, destination, !args.cosmos && !args.stellar)
    };

    let id = match &args.chain_id {
        Some(id) => id.trim().to_string(),
        None => chain_id(cfg, chain, destination),
    };

    if id.is_empty() {
        bail!("no source chain id — set STELLAR_CHAIN_ID / COSMOS_CHAIN_ID or pass --chain-id");
    }

    Ok(Source {
        chain,
        destination,
        id,
        inferred,
    })
}

fn chain_id(cfg: &Config, chain: Chain, destination: Chain) -> String {
    match (chain, destination) {
        (Chain::Evm, _) => EvmCorridorConfig::from_env().evm_chain_id,
        (Chain::Stellar, Chain::Evm) => EvmCorridorConfig::from_env().stellar_chain_id,
        (Chain::Cosmos, _) => cfg.cosmos.chain_id.as_str().to_string(),
        _ => cfg.stellar.chain_id.as_str().to_string(),
    }
}

fn infer_chain(tx: &str) -> Option<Chain> {
    let hash = tx.trim();

    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let upper = hash.chars().any(|c| c.is_ascii_uppercase());
    let lower = hash.chars().any(|c| c.is_ascii_lowercase());

    match (upper, lower) {
        (true, false) => Some(Chain::Cosmos),
        (false, true) => Some(Chain::Stellar),
        _ => None,
    }
}

fn unreachable_api(rc: &RelayConfig, error: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "could not reach ipsa-api at {} ({error}) — start it with `ipsa up`",
        rc.api_url
    )
}

fn relay_error(rc: &RelayConfig, status: reqwest::StatusCode, payload: &Value) -> anyhow::Error {
    let message = payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error");

    let hint = match status {
        reqwest::StatusCode::BAD_REQUEST if message.contains("unsupported chain") => Some(format!(
            "the relayer built no bridge client for it — check `chains` in {} lists the id under a chain type the relayer supports, then `ipsa services restart --relayer`",
            rc.relayer_config
        )),
        reqwest::StatusCode::NOT_FOUND => {
            Some("the tx is not visible on chain yet — retry once it is included".to_string())
        }
        reqwest::StatusCode::CONFLICT => {
            Some("the source tx failed on chain, so there is no packet to relay".to_string())
        }
        reqwest::StatusCode::BAD_GATEWAY => Some(
            "the relayer api is down — start it with `ipsa services up --relayer`"
                .to_string(),
        ),
        _ => None,
    };

    match hint {
        Some(hint) => anyhow::anyhow!("relay rejected: {message}\n  → {hint}"),
        None => anyhow::anyhow!("relay rejected: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{infer_chain, outcome, Outcome, Packet};
    use crate::shared::Chain;

    #[test]
    fn infers_stellar_from_a_lowercase_hash() {
        let hash = "ab".repeat(32);

        assert!(matches!(infer_chain(&hash), Some(Chain::Stellar)));
    }

    #[test]
    fn infers_cosmos_from_an_uppercase_hash() {
        let hash = "AB".repeat(32);

        assert!(matches!(infer_chain(&hash), Some(Chain::Cosmos)));
    }

    #[test]
    fn refuses_to_guess_on_ambiguous_hashes() {
        assert!(infer_chain(&"12".repeat(32)).is_none());
        assert!(infer_chain(&"aB".repeat(32)).is_none());
        assert!(infer_chain("deadbeef").is_none());
        assert!(infer_chain(&"zz".repeat(32)).is_none());
    }

    fn packet(state: &str) -> Packet {
        Packet {
            state: state.to_string(),
            sequence: 1,
            source_client_id: "07-tendermint-0".to_string(),
            recv: None,
            ack: None,
            timeout: None,
        }
    }

    #[test]
    fn only_terminal_states_count_as_settled() {
        assert!(packet("COMPLETE").settled());
        assert!(packet("FAILED").settled());
        assert!(!packet("PENDING").settled());
        assert!(!packet("UNKNOWN").settled());
    }

    #[test]
    fn every_packet_must_settle_before_the_relay_is_done() {
        assert!(matches!(
            outcome(&[packet("COMPLETE"), packet("PENDING")]),
            Outcome::Pending
        ));
        assert!(matches!(
            outcome(&[packet("COMPLETE"), packet("COMPLETE")]),
            Outcome::Relayed { total: 2 }
        ));
    }

    #[test]
    fn an_empty_report_is_not_a_finished_relay() {
        assert!(matches!(outcome(&[]), Outcome::Pending));
    }

    #[test]
    fn a_failed_packet_fails_the_relay() {
        assert!(matches!(
            outcome(&[packet("COMPLETE"), packet("FAILED")]),
            Outcome::Failed {
                failed: 1,
                total: 2
            }
        ));
    }
}
