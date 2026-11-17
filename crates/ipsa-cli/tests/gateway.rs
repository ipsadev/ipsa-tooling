use ipsa::gateway::stellar::QuorumConfig;
use ipsa_cli::gateway::{
    describe_quorum_set, describe_trust_root, verify_quorum_fingerprints, with_scheme,
};
use sha2::{Digest, Sha256};
use stellar_xdr::{Limits, NodeId, PublicKey, ScpQuorumSet, Uint256, WriteXdr};

fn config(xdr: &[u8]) -> QuorumConfig {
    QuorumConfig {
        quorum_set_xdr: xdr.to_vec(),
        valid_from: 0,
    }
}

fn quorum_set() -> ScpQuorumSet {
    let node = NodeId(PublicKey::PublicKeyTypeEd25519(Uint256([7u8; 32])));

    ScpQuorumSet {
        threshold: 1,
        validators: vec![node].try_into().unwrap(),
        inner_sets: Default::default(),
    }
}

fn pin(xdr: &[u8]) -> String {
    hex::encode::<[u8; 32]>(Sha256::digest(xdr).into())
}

#[test]
fn a_bare_host_port_gets_a_scheme() {
    assert_eq!(with_scheme("127.0.0.1:50052"), "http://127.0.0.1:50052");
    assert_eq!(with_scheme(" gateway:50052/ "), "http://gateway:50052");
}

#[test]
fn an_explicit_scheme_is_left_alone() {
    assert_eq!(with_scheme("https://g:50052"), "https://g:50052");
}

#[test]
fn a_pinned_quorum_set_passes() {
    assert!(verify_quorum_fingerprints(&[config(b"abc")], &[pin(b"abc")]).is_ok());
}

#[test]
fn an_unpinned_quorum_set_is_refused() {
    assert!(verify_quorum_fingerprints(&[config(b"evil")], &[pin(b"abc")]).is_err());
}

#[test]
fn no_pins_at_all_is_refused() {
    assert!(verify_quorum_fingerprints(&[config(b"abc")], &[]).is_err());
}

#[test]
fn every_config_must_be_pinned_not_just_one() {
    let pins = vec![pin(b"abc")];

    assert!(verify_quorum_fingerprints(&[config(b"abc"), config(b"evil")], &pins).is_err());
}

#[test]
fn a_malformed_pin_is_refused() {
    assert!(verify_quorum_fingerprints(&[config(b"abc")], &["zz".to_string()]).is_err());
    assert!(verify_quorum_fingerprints(&[config(b"abc")], &["ab".to_string()]).is_err());
}

#[test]
fn the_decoded_root_lists_the_threshold_and_validators() {
    let lines = describe_quorum_set(&quorum_set(), 1);

    assert!(lines[0].contains("threshold 1 of 1 validator(s) and 0 inner set(s)"));

    let validator = lines
        .iter()
        .map(|line| line.trim())
        .find(|line| line.starts_with('G'))
        .expect("a validator strkey line");

    assert_eq!(validator.len(), 56);
}

#[test]
fn describe_trust_root_accepts_a_real_quorum_set() {
    let xdr = quorum_set().to_xdr(Limits::none()).unwrap();

    assert!(describe_trust_root(&[config(&xdr)]).is_ok());
}

#[test]
fn describe_trust_root_refuses_bytes_that_are_not_a_quorum_set() {
    assert!(describe_trust_root(&[config(b"not xdr")]).is_err());
}
