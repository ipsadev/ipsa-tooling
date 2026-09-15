use std::{str::FromStr, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use cosmrs::{
    crypto::secp256k1::SigningKey,
    proto::cosmos::{
        bank::v1beta1::MsgSend,
        base::v1beta1::Coin as ProtoCoin,
        gov::v1::{MsgSubmitProposal, MsgVote},
    },
    tendermint::chain::Id as ChainId,
    tx::{Body, Fee, SignDoc, SignerInfo},
    Any, Coin,
};
use ibc_proto::ibc::lightclients::wasm::v1::MsgStoreCode;
use prost13::Message;
use serde_json::Value;

use crate::services::cosmos::config::CosmosConfig;

struct Account {
    key: SigningKey,
    address: String,
}

pub fn address_from_key(hex_str: &str, prefix: &str) -> Result<String> {
    let account = load_key(hex_str, prefix, "COSMOS_RELAYER_PRIVATE_KEY")?
        .ok_or_else(|| anyhow!("COSMOS_RELAYER_PRIVATE_KEY is empty"))?;

    Ok(account.address)
}

fn load_key(hex_str: &str, prefix: &str, label: &str) -> Result<Option<Account>> {
    let trimmed = hex_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let bytes = hex::decode(trimmed).with_context(|| format!("{label} is not valid hex"))?;
    let key = SigningKey::from_slice(&bytes)
        .map_err(|e| anyhow!("invalid {label} secp256k1 key: {e}"))?;
    let address = key
        .public_key()
        .account_id(prefix)
        .map_err(|e| anyhow!("derive {label} account id: {e}"))?
        .to_string();
    Ok(Some(Account { key, address }))
}

struct AccountInfo {
    account_number: u64,
    sequence: u64,
}

struct BroadcastResult {
    tx_hash: String,
    code: u32,
    raw_log: String,
}

pub struct CosmosSigner {
    http: reqwest::Client,
    rest_url: String,
    chain_id: String,
    gas_denom: String,
    relayer: Option<Account>,
    validator: Option<Account>,
}

impl CosmosSigner {
    pub fn from_config(cfg: &CosmosConfig, http: reqwest::Client) -> Result<Self> {
        Ok(Self {
            http,
            rest_url: cfg.rest_url.clone(),
            chain_id: cfg.chain_id.as_str().to_string(),
            gas_denom: cfg.gas_denom.clone(),
            relayer: load_key(
                &cfg.relayer_key_hex,
                &cfg.account_prefix,
                "COSMOS_RELAYER_PRIVATE_KEY",
            )?,
            validator: load_key(
                &cfg.validator_key_hex,
                &cfg.account_prefix,
                "COSMOS_VALIDATOR_PRIVATE_KEY",
            )?,
        })
    }

    pub fn relayer_address(&self) -> Result<&str> {
        self.relayer
            .as_ref()
            .map(|a| a.address.as_str())
            .ok_or_else(|| anyhow!("COSMOS_RELAYER_PRIVATE_KEY not configured"))
    }

    fn relayer(&self) -> Result<&Account> {
        self.relayer
            .as_ref()
            .ok_or_else(|| anyhow!("COSMOS_RELAYER_PRIVATE_KEY not configured"))
    }

    fn validator(&self) -> Result<&Account> {
        self.validator
            .as_ref()
            .ok_or_else(|| anyhow!("COSMOS_VALIDATOR_PRIVATE_KEY not configured"))
    }

    fn rest(&self, path: &str) -> String {
        format!("{}{}", self.rest_url.trim_end_matches('/'), path)
    }

    pub async fn node_info_ok(&self) -> bool {
        let url = self.rest("/cosmos/base/tendermint/v1beta1/node_info");
        matches!(self.http.get(&url).send().await, Ok(r) if r.status().is_success())
    }

    pub async fn checksums(&self) -> Result<Vec<String>> {
        let url = self.rest("/ibc/lightclients/wasm/v1/checksums");
        let body: Value = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(body
            .get("checksums")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn account_exists(&self, address: &str) -> Result<bool> {
        let url = self.rest(&format!("/cosmos/auth/v1beta1/accounts/{address}"));
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        resp.error_for_status()?;
        Ok(true)
    }

    async fn account_info(&self, address: &str) -> Result<AccountInfo> {
        let url = self.rest(&format!("/cosmos/auth/v1beta1/accounts/{address}"));
        let body: Value = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let account = body
            .get("account")
            .ok_or_else(|| anyhow!("response missing 'account': {body}"))?;
        let account_number = account
            .get("account_number")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| anyhow!("account_number missing: {account}"))?;
        let sequence = account
            .get("sequence")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(AccountInfo {
            account_number,
            sequence,
        })
    }

    async fn gov_module_address(&self) -> Result<String> {
        let url = self.rest("/cosmos/auth/v1beta1/module_accounts/gov");
        let body: Value = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        body.pointer("/account/value/address")
            .or_else(|| body.pointer("/account/base_account/address"))
            .or_else(|| body.pointer("/account/address"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("gov module address not found in response: {body}"))
    }

    async fn tx_by_hash(&self, tx_hash: &str) -> Result<Value> {
        let url = self.rest(&format!("/cosmos/tx/v1beta1/txs/{tx_hash}"));
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            bail!("tx not found");
        }
        Ok(resp.error_for_status()?.json().await?)
    }

    async fn wait_for_tx(&self, tx_hash: &str, timeout: Duration) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut delay = Duration::from_millis(500);
        loop {
            match self.tx_by_hash(tx_hash).await {
                Ok(v) => return Ok(v),
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(delay).await;
                    if delay < Duration::from_secs(2) {
                        delay *= 2;
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn sign_and_broadcast(
        &self,
        account: &Account,
        msg_any: Any,
        memo: &str,
        gas_limit: u64,
        fee_amount: u128,
    ) -> Result<BroadcastResult> {
        let info = self.account_info(&account.address).await?;

        let body = Body::new(vec![msg_any], memo, 0u32);
        let fee = Fee::from_amount_and_gas(
            Coin {
                denom: cosmrs::Denom::from_str(&self.gas_denom)
                    .map_err(|e| anyhow!("invalid gas denom: {e}"))?,
                amount: fee_amount,
            },
            gas_limit,
        );
        let auth_info =
            SignerInfo::single_direct(Some(account.key.public_key()), info.sequence).auth_info(fee);
        let chain_id = ChainId::try_from(self.chain_id.clone())
            .map_err(|e| anyhow!("invalid chain id: {e}"))?;
        let sign_doc = SignDoc::new(&body, &auth_info, &chain_id, info.account_number)
            .map_err(|e| anyhow!("build sign doc: {e}"))?;
        let tx_raw_bytes = sign_doc
            .sign(&account.key)
            .map_err(|e| anyhow!("sign tx: {e}"))?
            .to_bytes()
            .map_err(|e| anyhow!("encode tx_raw: {e}"))?;

        self.broadcast_tx_bytes(tx_raw_bytes).await
    }

    async fn broadcast_tx_bytes(&self, tx_bytes: Vec<u8>) -> Result<BroadcastResult> {
        let url = self.rest("/cosmos/tx/v1beta1/txs");
        let body = serde_json::json!({
            "tx_bytes": BASE64.encode(&tx_bytes),
            "mode": "BROADCAST_MODE_SYNC",
        });
        let resp = self.http.post(&url).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("broadcast failed ({status}): {text}");
        }
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("non-json broadcast response: {text}"))?;
        let tx_response = value
            .get("tx_response")
            .ok_or_else(|| anyhow!("missing tx_response: {value}"))?;
        Ok(BroadcastResult {
            tx_hash: tx_response
                .get("txhash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            code: tx_response
                .get("code")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            raw_log: tx_response
                .get("raw_log")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
    }

    pub async fn fund_account(
        &self,
        to: &str,
        amount: u128,
        gas_limit: u64,
        fee_amount: u128,
        skip_if_exists: bool,
    ) -> Result<bool> {
        if skip_if_exists && self.account_exists(to).await? {
            return Ok(false);
        }

        let validator = self.validator()?;
        let msg = MsgSend {
            from_address: validator.address.clone(),
            to_address: to.to_string(),
            amount: vec![ProtoCoin {
                denom: self.gas_denom.clone(),
                amount: amount.to_string(),
            }],
        };
        let msg_any = Any {
            type_url: "/cosmos.bank.v1beta1.MsgSend".to_string(),
            value: msg.encode_to_vec(),
        };
        let result = self
            .sign_and_broadcast(validator, msg_any, "bank-send", gas_limit, fee_amount)
            .await?;
        if result.code != 0 {
            bail!(
                "bank send rejected (code {}): {}",
                result.code,
                result.raw_log
            );
        }
        Ok(true)
    }

    pub async fn broadcast_as_relayer(
        &self,
        msg: Any,
        memo: &str,
        gas_limit: u64,
        fee_amount: u128,
        wait_timeout: Duration,
    ) -> Result<Value> {
        let relayer = self.relayer()?;

        let result = self
            .sign_and_broadcast(relayer, msg, memo, gas_limit, fee_amount)
            .await?;

        if result.code != 0 {
            bail!(
                "{memo} broadcast rejected (code {}): {}",
                result.code,
                result.raw_log
            );
        }

        let landed = self.wait_for_tx(&result.tx_hash, wait_timeout).await?;
        let landed_code = landed
            .pointer("/tx_response/code")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if landed_code != 0 {
            let raw_log = landed
                .pointer("/tx_response/raw_log")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            bail!(
                "{memo} failed on chain (code {landed_code}, hash {}): {raw_log}",
                result.tx_hash
            );
        }

        Ok(landed)
    }

    pub async fn submit_store_code(
        &self,
        wasm_bytes: Vec<u8>,
        title: String,
        summary: String,
        deposit_amount: u128,
        gas_limit: u64,
        fee_amount: u128,
        wait_timeout: Duration,
    ) -> Result<u64> {
        let gov_addr = self.gov_module_address().await?;
        let relayer = self.relayer()?;

        let store_code = MsgStoreCode {
            signer: gov_addr,
            wasm_byte_code: wasm_bytes,
        };
        let msg = MsgSubmitProposal {
            messages: vec![cosmrs::proto::Any {
                type_url: "/ibc.lightclients.wasm.v1.MsgStoreCode".to_string(),
                value: store_code.encode_to_vec(),
            }],
            initial_deposit: vec![ProtoCoin {
                denom: self.gas_denom.clone(),
                amount: deposit_amount.to_string(),
            }],
            proposer: relayer.address.clone(),
            metadata: String::new(),
            title,
            summary,
            expedited: false,
        };
        let msg_any = Any {
            type_url: "/cosmos.gov.v1.MsgSubmitProposal".to_string(),
            value: msg.encode_to_vec(),
        };

        let result = self
            .sign_and_broadcast(relayer, msg_any, "upload-lc-wasm", gas_limit, fee_amount)
            .await?;
        if result.code != 0 {
            bail!(
                "store-code broadcast rejected (code {}): {}",
                result.code,
                result.raw_log
            );
        }

        let landed = self.wait_for_tx(&result.tx_hash, wait_timeout).await?;
        let landed_code = landed
            .pointer("/tx_response/code")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if landed_code != 0 {
            let raw_log = landed
                .pointer("/tx_response/raw_log")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            bail!(
                "store-code tx failed on-chain (code {landed_code}, hash {}): {raw_log}",
                result.tx_hash
            );
        }

        extract_proposal_id(&landed).ok_or_else(|| {
            anyhow!(
                "proposal_id not found in landed tx {} events",
                result.tx_hash
            )
        })
    }

    pub async fn vote(
        &self,
        proposal_id: u64,
        option: i32,
        gas_limit: u64,
        fee_amount: u128,
    ) -> Result<()> {
        let voter = self.validator().or_else(|_| self.relayer())?;
        let msg = MsgVote {
            proposal_id,
            voter: voter.address.clone(),
            option,
            metadata: String::new(),
        };
        let msg_any = Any {
            type_url: "/cosmos.gov.v1.MsgVote".to_string(),
            value: msg.encode_to_vec(),
        };
        let result = self
            .sign_and_broadcast(voter, msg_any, "upload-lc-wasm-vote", gas_limit, fee_amount)
            .await?;
        if result.code != 0 {
            bail!("vote rejected (code {}): {}", result.code, result.raw_log);
        }
        Ok(())
    }
}

fn extract_proposal_id(landed: &Value) -> Option<u64> {
    let events = landed
        .pointer("/tx_response/events")
        .or_else(|| landed.get("events"))?;
    for event in events.as_array()? {
        if event.get("type").and_then(|v| v.as_str()) == Some("submit_proposal") {
            for attr in event.get("attributes")?.as_array()? {
                if attr.get("key").and_then(|v| v.as_str()) == Some("proposal_id") {
                    if let Some(id) = attr
                        .get("value")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        return Some(id);
                    }
                }
            }
        }
    }
    None
}
