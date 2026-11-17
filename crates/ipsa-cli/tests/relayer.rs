use ipsa_cli::{
    services::relayer::wiring::{with_field, with_route},
    tests::relayer_routes::names_client,
};

const CONFIG: &str = "chains:\n  simd-1:\n    type: 'cosmos'\n    ibcv2:\n      counterparty_chains:\n        08-wasm-2: 'stellar-testnet'\n\n  stellar-testnet:\n    type: 'stellar'\n    ibcv2:\n      counterparty_chains:\n        07-tendermint-6: 'simd-1'\n";

#[test]
fn a_new_client_id_replaces_the_route_to_the_same_chain() {
    let out = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

    assert!(out.contains("        08-wasm-30: 'stellar-testnet'"));
    assert!(!out.contains("08-wasm-2:"));
}

#[test]
fn the_other_chains_table_is_left_alone() {
    let out = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

    assert!(out.contains("        07-tendermint-6: 'simd-1'"));
}

#[test]
fn writing_the_same_route_twice_changes_nothing() {
    let once = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();
    let twice = with_route(&once, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

    assert_eq!(once, twice);
}

#[test]
fn a_table_with_no_route_to_that_chain_gains_one() {
    let config = "chains:\n  simd-1:\n    ibcv2:\n      counterparty_chains:\n";
    let out = with_route(config, "simd-1", "08-wasm-1", "stellar-testnet").unwrap();

    assert!(out.contains("        08-wasm-1: 'stellar-testnet'"));
}

#[test]
fn a_chain_block_is_found_by_its_chain_id_when_its_key_differs() {
    let config = "chains:\n  stellar-testnet:\n    chain_id: 'stellar-testnet'\n    ibcv2:\n      counterparty_chains: {}\n  sepolia:\n    chain_id: '11155111'\n    ibcv2:\n      counterparty_chains: {}\n";
    let out = with_route(config, "11155111", "stellar-testnet-2", "stellar-testnet").unwrap();

    assert!(out.contains("  sepolia:\n    chain_id: '11155111'\n    ibcv2:\n      counterparty_chains:\n        stellar-testnet-2: 'stellar-testnet'"));
    assert!(out.contains("  stellar-testnet:\n    chain_id: 'stellar-testnet'\n    ibcv2:\n      counterparty_chains: {}"));
}

#[test]
fn an_unknown_chain_is_an_error_not_a_silent_no_op() {
    assert!(with_route(CONFIG, "osmosis-1", "08-wasm-1", "stellar-testnet").is_err());
}

#[test]
fn an_empty_inline_table_is_expanded_to_hold_the_route() {
    let config = "chains:\n  simd-1:\n    ibcv2:\n      counterparty_chains: {}\n";

    let out = with_route(config, "simd-1", "08-wasm-0", "stellar-testnet").unwrap();

    assert_eq!(
        out,
        "chains:\n  simd-1:\n    ibcv2:\n      counterparty_chains:\n        08-wasm-0: 'stellar-testnet'\n"
    );
}

#[test]
fn a_chain_without_the_table_is_an_error() {
    let config = "chains:\n  simd-1:\n    type: 'cosmos'\n";

    assert!(with_route(config, "simd-1", "08-wasm-1", "stellar-testnet").is_err());
}

#[test]
fn a_contract_id_is_replaced_in_place() {
    let config = "chains:\n  stellar-testnet:\n    stellar:\n      router_contract_id: 'COLD'\n";
    let out = with_field(config, "stellar-testnet", "router_contract_id", "CNEW").unwrap();

    assert!(out.contains("      router_contract_id: 'CNEW'"));
    assert!(!out.contains("COLD"));
}

#[test]
fn a_missing_field_is_an_error_not_an_invented_one() {
    let config = "chains:\n  stellar-testnet:\n    stellar:\n";

    assert!(with_field(config, "stellar-testnet", "router_contract_id", "C").is_err());
}

#[test]
fn the_trailing_newline_survives() {
    let out = with_route(CONFIG, "simd-1", "08-wasm-30", "stellar-testnet").unwrap();

    assert!(out.ends_with('\n'));
}

#[test]
fn a_listed_client_is_routable() {
    let config = "      counterparty_chains:\n        08-wasm-2: 'stellar-testnet'\n";

    assert!(names_client(config, "08-wasm-2"));
}

#[test]
fn an_unlisted_client_is_not() {
    let config = "      counterparty_chains:\n        08-wasm-2: 'stellar-testnet'\n";

    assert!(!names_client(config, "08-wasm-17"));
}

#[test]
fn a_prefix_of_a_listed_client_does_not_count() {
    let config = "      counterparty_chains:\n        08-wasm-2: 'stellar-testnet'\n";

    assert!(!names_client(config, "08-wasm-20"));
    assert!(!names_client("        08-wasm-20: 'x'", "08-wasm-2"));
}
