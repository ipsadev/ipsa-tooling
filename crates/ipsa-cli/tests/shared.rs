use ipsa_cli::shared::{chain_of, Chain};

#[test]
fn classifies_cosmos_bech32() {
    let addr = "cosmos1qy352eufqy352eufqy352eufqy35qqqqqqqqqq";

    assert!(matches!(chain_of(addr), Some(Chain::Cosmos)));
}

#[test]
fn classifies_stellar_account_and_contract() {
    let account = format!("G{}", "A".repeat(55));
    let contract = format!("C{}", "B".repeat(55));

    assert!(matches!(chain_of(&account), Some(Chain::Stellar)));
    assert!(matches!(chain_of(&contract), Some(Chain::Stellar)));
}

#[test]
fn trims_whitespace_before_classifying() {
    let account = format!("  G{}  ", "A".repeat(55));

    assert!(matches!(chain_of(&account), Some(Chain::Stellar)));
}

#[test]
fn rejects_unknown_addresses() {
    assert!(chain_of("").is_none());
    assert!(chain_of("0xabc123").is_none());
    assert!(chain_of(&format!("g{}", "a".repeat(55))).is_none());
    assert!(chain_of(&format!("G{}", "A".repeat(40))).is_none());
}
