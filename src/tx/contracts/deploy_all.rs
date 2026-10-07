use std::path::Path;

use anyhow::{bail, Result};

use crate::{config::ClientTypes, logger, tx::contracts::config::ContractsConfig};

pub fn run(
    cfg: &ContractsConfig,
    root: &Path,
    force: bool,
    attestation: bool,
    mock: bool,
) -> Result<bool> {
    logger::banner("contracts deploy-all (build + deploy + wire router + write .env)");

    if cfg.signing_key.is_empty() {
        bail!("STELLAR_SIGNING_KEY is empty in .env — generate + fund a testnet key and set it");
    }

    if !cfg.ibc_router.is_empty() && !force {
        logger::warn(&format!(
            "ROUTER_CONTRACT_ADDRESS already set ({}). Use --force to redeploy.",
            cfg.ibc_router
        ));

        return Ok(false);
    }

    let genesis = super::latest_ledger(cfg, root)?;
    logger::detail(&format!("router genesis ledger: {genesis}"));

    let wasm_dir = super::release::fetch(root)?;

    let deployer = deployer_address(cfg)?;
    logger::detail(&format!("deployer: {deployer}"));

    let wasm = |name: &str| wasm_dir.join(name).display().to_string();

    let mut mock_id = String::new();
    if mock {
        logger::warn(
            "--mock: deploying the mock light client, whose verify_membership accepts every proof. \
             Devnet only. Never deploy this where anything of value moves.",
        );
        logger::step("deploying mock light client");
        mock_id = super::deploy(cfg, root, &wasm("stellar_mock_light_client.wasm"), &[])?;
        logger::ok(&format!("mock LC: {mock_id}"));
    }

    logger::step("deploying router");
    let router = super::deploy(
        cfg,
        root,
        &wasm("stellar_ibc_router.wasm"),
        &["--owner", &deployer],
    )?;
    logger::ok(&format!("router: {router}"));

    logger::step("deploying transfer-app");
    let transfer = super::deploy(
        cfg,
        root,
        &wasm("stellar_ibc_transfer.wasm"),
        &["--router", &router, "--owner", &deployer],
    )?;
    logger::ok(&format!("transfer-app: {transfer}"));

    logger::step("deploying gmp-app");
    let gmp = super::deploy(
        cfg,
        root,
        &wasm("stellar_ibc_gmp.wasm"),
        &["--owner", &deployer, "--router", &router],
    )?;
    logger::ok(&format!("gmp-app: {gmp}"));

    let mut attestation_id = String::new();
    if attestation {
        logger::step("deploying attestation light client");
        attestation_id = super::deploy(
            cfg,
            root,
            &wasm("stellar_attestation_light_client.wasm"),
            &[],
        )?;
        logger::ok(&format!("attestation LC: {attestation_id}"));
    }

    logger::step("deploying tendermint light client");
    let tendermint_id = super::deploy(
        cfg,
        root,
        &wasm("stellar_tendermint_light_client.wasm"),
        &[],
    )?;
    logger::ok(&format!("tendermint LC: {tendermint_id}"));

    logger::step("wiring router (register_client_type + register_port)");
    if !mock_id.is_empty() {
        super::invoke(
            cfg,
            root,
            &router,
            &[
                "register_client_type",
                "--client_type",
                cfg.client_type(ClientTypes::Mock),
                "--light_client_address",
                &mock_id,
            ],
        )?;
    }

    if !attestation_id.is_empty() {
        super::invoke(
            cfg,
            root,
            &router,
            &[
                "register_client_type",
                "--client_type",
                cfg.client_type(ClientTypes::Attestation),
                "--light_client_address",
                &attestation_id,
            ],
        )?;
    }

    super::invoke(
        cfg,
        root,
        &router,
        &[
            "register_client_type",
            "--client_type",
            cfg.client_type(ClientTypes::Tendermint),
            "--light_client_address",
            &tendermint_id,
        ],
    )?;

    super::invoke(
        cfg,
        root,
        &router,
        &[
            "register_port",
            "--port_id",
            &cfg.transfer_port,
            "--app_address",
            &transfer,
        ],
    )?;

    super::invoke(
        cfg,
        root,
        &router,
        &[
            "register_port",
            "--port_id",
            "gmpport",
            "--app_address",
            &gmp,
        ],
    )?;

    logger::step("wiring gmp account factory (upload gmp-account wasm + set_account_wasm)");
    let account_hash = super::upload::hash(cfg, root, &wasm("ipsa_gmp_account.wasm"))?;
    logger::detail(&format!("gmp account wasm hash: {account_hash}"));
    super::invoke(
        cfg,
        root,
        &gmp,
        &["set_account_wasm", "--wasm_hash", &account_hash],
    )?;

    logger::step("wiring voucher factory (upload voucher-token wasm + set_voucher_wasm)");
    let voucher_hash = super::upload::hash(cfg, root, &wasm("stellar_ibc_voucher_token.wasm"))?;
    logger::detail(&format!("voucher wasm hash: {voucher_hash}"));
    super::invoke(
        cfg,
        root,
        &transfer,
        &["set_voucher_wasm", "--wasm_hash", &voucher_hash],
    )?;

    logger::step("registering transfer assets");
    crate::tx::transfer::assets::register_all(cfg, root, &transfer)?;

    logger::step("writing contract ids to relayer.yml");
    wire_relayer(cfg, &router, &transfer);

    logger::ok("deploy-all complete");

    let genesis = genesis.to_string();
    let addresses = [
        ("ROUTER_CONTRACT_ADDRESS", router.as_str()),
        ("TRANSFER_CONTRACT_ADDRESS", transfer.as_str()),
        ("GMP_CONTRACT_ADDRESS", gmp.as_str()),
        ("MOCK_LC_CONTRACT_ID", mock_id.as_str()),
        ("ATTESTATION_LC_CONTRACT_ID", attestation_id.as_str()),
        ("TENDERMINT_LC_CONTRACT_ID", tendermint_id.as_str()),
        ("DEPLOYER_ADDRESS", deployer.as_str()),
        ("ROUTER_GENESIS_LEDGER", genesis.as_str()),
    ];

    let env = root.join(".env");
    logger::step(&format!("writing deployed addresses to {}", env.display()));
    crate::shared::env_upsert(&env, &addresses)?;

    for (name, value) in addresses {
        logger::detail(&format!("{name}={value}"));
    }

    let stale = ["COSMOS_CLIENT_ID", "STELLAR_CLIENT_ID"]
        .into_iter()
        .filter(|name| !crate::config::get(name, "").is_empty())
        .collect::<Vec<_>>();

    if !stale.is_empty() {
        logger::warn(&format!(
            "{} still name clients on the previous router — recreate them with `ipsa tx clients create --force`",
            stale.join(", ")
        ));
    }

    logger::hint(
        "the api and gateway read these at startup: ipsa services restart --api --gateway",
    );

    Ok(true)
}

fn wire_relayer(cfg: &ContractsConfig, router: &str, transfer: &str) {
    let path = Path::new(&cfg.relayer_config);

    for (key, value) in [
        ("router_contract_id", router),
        ("transfer_contract_id", transfer),
    ] {
        match crate::services::relayer::wiring::set_field(path, &cfg.stellar_chain_id, key, value) {
            Ok(true) => logger::detail(&format!("{}: {key} = {value}", path.display())),
            Ok(false) => logger::detail(&format!("{} already had {key}", path.display())),
            Err(error) => logger::warn(&format!(
                "could not set {key} in {}: {error:#} — the ibc-relayer will read no packets from \
                 the router until it matches",
                path.display()
            )),
        }
    }

    logger::hint("the relayer reads that file at startup: ipsa services restart --relayer");
}

pub(crate) fn deployer_address(cfg: &ContractsConfig) -> Result<String> {
    if !cfg.deployer_address.is_empty() {
        return Ok(cfg.deployer_address.clone());
    }

    address_from_secret(&cfg.signing_key)
}

fn address_from_secret(secret: &str) -> Result<String> {
    let private = stellar_strkey::ed25519::PrivateKey::from_string(secret)
        .map_err(|e| anyhow::anyhow!("invalid STELLAR_SIGNING_KEY: {e}"))?;
    let signing = ed25519_dalek::SigningKey::from_bytes(&private.0);
    let public = stellar_strkey::ed25519::PublicKey(signing.verifying_key().to_bytes());

    Ok(String::from(public.to_string().as_str()))
}

#[cfg(test)]
mod tests {
    use super::address_from_secret;

    #[test]
    fn derives_the_stellar_address_from_the_secret() {
        let secret = "SCY47WDEEXHWBUD42ZYYG3DPIVX52RLQULF6KRPEEHLRU5TIWDKIJTGH";
        let address = "GCSWETSPE54NXRXLEXXI2FBIL2KRGOMF6JEJQ62OAGSXXOJVKO6WDAVO";

        assert_eq!(address_from_secret(secret).unwrap(), address);
    }
}
