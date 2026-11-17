use ipsa_cli::services::proof_api::forwarding_to;
use serde_json::Value;

const CONFIG: &str = r#"{
  "server": { "address": "0.0.0.0", "port": 3100 },
  "modules": [
    { "name": "stellar_to_eth", "src_chain": "stellar-testnet", "dst_chain": "11155111", "config": {} },
    { "name": "eth_to_stellar", "src_chain": "11155111", "dst_chain": "stellar-testnet", "config": {} },
    { "name": "forward", "src_chain": "stellar-testnet", "dst_chain": "placeholder", "config": { "target": "http://127.0.0.1:50052" } },
    { "name": "forward", "src_chain": "placeholder", "dst_chain": "stellar-testnet", "config": { "target": "http://127.0.0.1:50052" } }
  ]
}"#;

fn modules(config: &str) -> Vec<(String, String, String)> {
    let value: Value = serde_json::from_str(config).unwrap();

    value["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|module| {
            (
                module["name"].as_str().unwrap().to_string(),
                module["src_chain"].as_str().unwrap().to_string(),
                module["dst_chain"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn forward_modules_take_the_cosmos_chain_id_on_the_non_stellar_side() {
    let out = forwarding_to(CONFIG, "stellar-testnet", "simd-1").unwrap();
    let forward: Vec<_> = modules(&out)
        .into_iter()
        .filter(|(name, _, _)| name == "forward")
        .collect();

    assert!(forward
        .iter()
        .any(|(_, src, dst)| src == "stellar-testnet" && dst == "simd-1"));
    assert!(forward
        .iter()
        .any(|(_, src, dst)| src == "simd-1" && dst == "stellar-testnet"));
}

#[test]
fn the_non_forward_modules_are_left_alone() {
    let out = forwarding_to(CONFIG, "stellar-testnet", "simd-1").unwrap();

    for (name, src, dst) in modules(&out) {
        if name == "stellar_to_eth" {
            assert_eq!(
                (src.as_str(), dst.as_str()),
                ("stellar-testnet", "11155111")
            );
        }

        if name == "eth_to_stellar" {
            assert_eq!(
                (src.as_str(), dst.as_str()),
                ("11155111", "stellar-testnet")
            );
        }
    }
}
