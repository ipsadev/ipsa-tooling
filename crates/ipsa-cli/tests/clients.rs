use cosmrs::{proto::cosmos::tx::v1beta1::TxBody, Any};
use ibc_proto::{
    google::protobuf::Any as IbcAny,
    ibc::{
        core::client::v1::MsgCreateClient, lightclients::wasm::v1::ClientState as WasmClientState,
    },
};
use ipsa::gateway::stellar::StellarClientState;
use ipsa_cli::tx::clients::stellar::{
    client_id_from, expected_network_id, verify_stellar_client_state,
};
use prost13::Message;
use serde_json::json;

fn create_client_tx(router: [u8; 32], topic: &[u8], network_id: [u8; 32]) -> Vec<u8> {
    let stellar = StellarClientState {
        chain_id: "stellar-testnet".to_string(),
        latest_height: None,
        frozen_height: None,
        quorum_configs: Vec::new(),
        proof_specs: Vec::new(),
        network_id: network_id.to_vec(),
        max_consensus_age: 0,
        router_contract_id: router.to_vec(),
        root_event_topic: topic.to_vec(),
    };

    let wasm = WasmClientState {
        data: prost::Message::encode_to_vec(&stellar),
        checksum: Vec::new(),
        latest_height: None,
    };

    let create = MsgCreateClient {
        client_state: Some(IbcAny {
            type_url: "/ibc.lightclients.wasm.v1.ClientState".to_string(),
            value: wasm.encode_to_vec(),
        }),
        consensus_state: None,
        signer: "relayer".to_string(),
    };

    let body = TxBody {
        messages: vec![Any {
            type_url: "/ibc.core.client.v1.MsgCreateClient".to_string(),
            value: create.encode_to_vec(),
        }],
        ..Default::default()
    };

    body.encode_to_vec()
}

#[test]
fn a_matching_create_client_tx_is_accepted() {
    let router = [7u8; 32];
    let strkey = stellar_strkey::Contract(router).to_string();
    let network = expected_network_id("stellar-testnet");

    let tx = create_client_tx(router, b"ibc_root", network);

    verify_stellar_client_state(&tx, &strkey, "stellar-testnet").unwrap();
}

#[test]
fn a_create_client_tx_for_another_router_is_refused() {
    let strkey = stellar_strkey::Contract([7u8; 32]).to_string();
    let network = expected_network_id("stellar-testnet");

    let tx = create_client_tx([9u8; 32], b"ibc_root", network);

    let error = verify_stellar_client_state(&tx, &strkey, "stellar-testnet").unwrap_err();

    assert!(error.to_string().contains("router_contract_id"));
}

#[test]
fn a_create_client_tx_with_a_foreign_root_topic_is_refused() {
    let router = [7u8; 32];
    let strkey = stellar_strkey::Contract(router).to_string();
    let network = expected_network_id("stellar-testnet");

    let tx = create_client_tx(router, b"not_ibc_root", network);

    let error = verify_stellar_client_state(&tx, &strkey, "stellar-testnet").unwrap_err();

    assert!(error.to_string().contains("root_event_topic"));
}

#[test]
fn the_client_id_comes_from_the_create_client_event() {
    let landed = json!({"tx_response": {"events": [
        {"type": "message", "attributes": [{"key": "client_id", "value": "wrong"}]},
        {"type": "create_client", "attributes": [
            {"key": "client_type", "value": "08-wasm"},
            {"key": "client_id", "value": "08-wasm-3"}
        ]}
    ]}});

    assert_eq!(client_id_from(&landed).unwrap(), "08-wasm-3");
}

#[test]
fn a_tx_without_a_create_client_event_yields_nothing() {
    let landed = json!({"tx_response": {"events": [{"type": "message", "attributes": []}]}});

    assert!(client_id_from(&landed).is_none());
}
