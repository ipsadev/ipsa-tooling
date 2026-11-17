use ipsa_cli::{
    shared::Chain,
    tx::relay::{
        budget::{archive_seconds, checkpoint_for, ledgers_to_checkpoint, ARCHIVE_MARGIN_SECS},
        infer_chain, is_db_error, outcome, relay_error, Outcome, Packet, RelayConfig,
    },
};
use serde_json::json;

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

#[test]
fn a_duplicate_key_reads_as_a_db_error() {
    assert!(is_db_error(
        "ERROR: duplicate key value violates unique constraint \"index_ibcv2_transfer_packet\" (SQLSTATE 23505)"
    ));
}

#[test]
fn a_connection_failure_is_not_a_db_error() {
    assert!(!is_db_error("connection refused"));
}

fn config() -> RelayConfig {
    RelayConfig {
        api_url: "http://localhost:8080".to_string(),
        relayer_config: "relayer.yml".to_string(),
    }
}

#[test]
fn a_relayer_db_error_is_shown_as_is() {
    let payload = json!({
        "error": "duplicate key value violates unique constraint \"index_ibcv2_transfer_packet\" (SQLSTATE 23505)"
    });

    let rendered = relay_error(&config(), reqwest::StatusCode::BAD_GATEWAY, &payload).to_string();

    assert!(rendered.contains("index_ibcv2_transfer_packet"));
    assert!(!rendered.contains("the relayer api is down"));
}

#[test]
fn an_unreachable_relayer_still_hints_to_start_it() {
    let payload = json!({ "error": "upstream connect error" });

    let rendered = relay_error(&config(), reqwest::StatusCode::BAD_GATEWAY, &payload).to_string();

    assert!(rendered.contains("the relayer api is down"));
}

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

#[test]
fn the_worst_case_window_outlasts_the_old_flat_default() {
    assert!(archive_seconds(64) > 300);
}
