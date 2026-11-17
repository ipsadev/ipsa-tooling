use ipsa_cli::tx::contracts::{deploy_all::address_from_secret, parse_sequence, tx_hash};

const HASH: &str = "3f1c2b8a9d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8";
const OTHER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn reads_the_signing_transaction_line() {
    let out = format!("ℹ️ Signing transaction: {HASH}\nsuccess\n");

    assert_eq!(tx_hash(&out).as_deref(), Some(HASH));
}

#[test]
fn reads_the_transaction_hash_line() {
    let out = format!("Transaction hash is {HASH}\n🔗 https://stellar.expert/tx/{HASH}\n");

    assert_eq!(tx_hash(&out).as_deref(), Some(HASH));
}

#[test]
fn takes_the_last_hash_when_a_restore_tx_ran_first() {
    let out = format!(
        "ℹ️ Signing transaction: {OTHER}\nℹ️ Transaction hash is {OTHER}\nℹ️ Signing transaction: {HASH}\nℹ️ Transaction hash is {HASH}\n"
    );

    assert_eq!(tx_hash(&out).as_deref(), Some(HASH));
}

#[test]
fn returns_none_when_nothing_was_submitted() {
    assert!(
        tx_hash("Simulation identified as read-only. Send by rerunning with --send=yes.").is_none()
    );
    assert!(tx_hash("").is_none());
    assert!(tx_hash("Signing transaction: not-a-hash").is_none());
    assert!(tx_hash(&format!("Signing transaction: {}", &HASH[..40])).is_none());
}

#[test]
fn the_sequence_line_is_read() {
    let out = "Sequence: 4358291\nProtocol Version: 27\nHash: bba34545\n";

    assert_eq!(parse_sequence(out), Some(4_358_291));
}

#[test]
fn output_without_a_sequence_yields_nothing() {
    assert_eq!(parse_sequence("Protocol Version: 27\n"), None);
}

#[test]
fn derives_the_stellar_address_from_the_secret() {
    let secret = "SCY47WDEEXHWBUD42ZYYG3DPIVX52RLQULF6KRPEEHLRU5TIWDKIJTGH";
    let address = "GCSWETSPE54NXRXLEXXI2FBIL2KRGOMF6JEJQ62OAGSXXOJVKO6WDAVO";

    assert_eq!(address_from_secret(secret).unwrap(), address);
}
