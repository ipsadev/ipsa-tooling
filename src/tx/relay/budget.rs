use super::RelayConfig;
use crate::{logger, shared::Chain};

const CHECKPOINT_FREQUENCY: u32 = 64;

const LEDGER_SECS: u64 = 5;

const ARCHIVE_MARGIN_SECS: u64 = 120;

const SERVED_SECS: u64 = 180;

const EVM_CORRIDOR_SECS: u64 = 3_600;

pub struct Budget {
    pub seconds: u64,
    pub reason: String,
}

pub async fn derive(
    rc: &RelayConfig,
    http: &reqwest::Client,
    source: Chain,
    destination: Chain,
    explicit: Option<u64>,
) -> Budget {
    if let Some(seconds) = explicit {
        return Budget {
            seconds,
            reason: "as asked".to_string(),
        };
    }

    if matches!(source, Chain::Evm) || matches!(destination, Chain::Evm) {
        return Budget {
            seconds: EVM_CORRIDOR_SECS,
            reason: "the EVM corridor waits for Ethereum finality and proves on the Succinct network, once for the packet and again for its acknowledgement".to_string(),
        };
    }

    if !matches!(source, Chain::Stellar) {
        return Budget {
            seconds: SERVED_SECS,
            reason: "the proof comes from Cosmos, which publishes it with the block".to_string(),
        };
    }

    let health = crate::probe::get_json(http, &format!("{}/watcher/health", rc.api_url)).await;

    let Some(health) = health else {
        return Budget {
            seconds: SERVED_SECS + ARCHIVE_MARGIN_SECS,
            reason: "could not read /watcher/health — assuming the archives".to_string(),
        };
    };

    let status = health
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if status == "healthy" {
        let ahead = health
            .get("ahead_of_archive")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        return Budget {
            seconds: SERVED_SECS,
            reason: format!("the watcher is serving, {ahead} ledgers ahead of the archive"),
        };
    }

    let live = health
        .get("live_ledger")
        .and_then(serde_json::Value::as_u64)
        .and_then(|l| u32::try_from(l).ok());

    let Some(live) = live else {
        return Budget {
            seconds: SERVED_SECS + ARCHIVE_MARGIN_SECS,
            reason: format!("the watcher is {status} and the chain tip is unknown"),
        };
    };

    let ledgers = ledgers_to_checkpoint(live);

    Budget {
        seconds: archive_seconds(live),
        reason: format!(
            "the watcher is {status}, so SCP evidence comes from the archives; ledger {live} \
             publishes at checkpoint {}, {ledgers} ledgers away",
            checkpoint_for(live)
        ),
    }
}

pub fn announce(budget: &Budget) {
    logger::detail(&format!(
        "{} — waiting up to {}s",
        budget.reason, budget.seconds
    ));
}

fn checkpoint_for(ledger: u32) -> u32 {
    ((ledger / CHECKPOINT_FREQUENCY) + 1) * CHECKPOINT_FREQUENCY - 1
}

fn ledgers_to_checkpoint(ledger: u32) -> u32 {
    checkpoint_for(ledger) - ledger
}

fn archive_seconds(live: u32) -> u64 {
    u64::from(ledgers_to_checkpoint(live)) * LEDGER_SECS + ARCHIVE_MARGIN_SECS
}

#[cfg(test)]
mod tests {
    use super::{archive_seconds, checkpoint_for, ledgers_to_checkpoint, ARCHIVE_MARGIN_SECS};

    #[test]
    fn a_checkpoint_is_the_last_ledger_of_its_64_wide_window() {
        assert_eq!(checkpoint_for(64), 127);
        assert_eq!(checkpoint_for(0), 63);
        assert_eq!(
            checkpoint_for(63),
            63,
            "a checkpoint ledger publishes itself"
        );
    }

    #[test]
    fn a_ledger_just_past_a_checkpoint_waits_almost_a_full_window() {
        assert_eq!(ledgers_to_checkpoint(64), 63);
    }

    #[test]
    fn a_ledger_on_a_checkpoint_boundary_waits_the_margin_only() {
        assert_eq!(archive_seconds(63), ARCHIVE_MARGIN_SECS);
    }

    // The whole point: the old flat 300s expired before a full window could close.
    #[test]
    fn the_worst_case_window_outlasts_the_old_flat_default() {
        assert!(archive_seconds(64) > 300);
    }
}
