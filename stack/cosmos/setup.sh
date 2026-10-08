#!/bin/sh
set -e

CONFIG_JSON="${COSMOS_CONFIG_JSON:-/config/default-config.json}"
HOME_DIR="${SIMD_HOME:-/root/.simapp}"
CONFIG_FOLDER="$HOME_DIR/config"
KEYRING="--keyring-backend test --home $HOME_DIR"

ensure_jq () {
    command -v jq >/dev/null 2>&1 && return
    apk add --no-cache jq 2>/dev/null && return
    (apt-get update && apt-get install -y --no-install-recommends jq) >/dev/null 2>&1
}

mnemonic_for () {
    case "$1" in
        val) printf '%s' "$COSMOS_VALIDATOR_MNEMONIC" ;;
        relayer) printf '%s' "$COSMOS_RELAYER_MNEMONIC" ;;
        *) printf '' ;;
    esac
}

add_account () {
    name="$1"
    strict="$2"

    mnemonic="$(mnemonic_for "$name" | tr -d "\"'\r")"
    mnemonic="$(printf '%s' "$mnemonic" | awk '{$1=$1};1')"

    if [ -z "$mnemonic" ]; then
        echo "error: no mnemonic in env for account '$name' — set COSMOS_VALIDATOR_MNEMONIC / COSMOS_RELAYER_MNEMONIC" >&2
        exit 1
    fi

    coins="$(jq -r --arg n "$name" '.accounts[$n]' "$CONFIG_JSON")"

    if ! echo "$mnemonic" | simd keys add "$name" --recover $KEYRING 2>&1; then
        if [ "$strict" = "strict" ]; then
            echo "error: failed to import key for account '$name'" >&2
            exit 1
        fi

        echo "warning: skipping account '$name' — its mnemonic duplicates an account already imported (validator and relayer share a mnemonic)" >&2

        return 0
    fi

    address="$(simd keys show "$name" -a $KEYRING)"

    if ! simd genesis add-genesis-account "$address" "$coins" --home "$HOME_DIR" 2>&1; then
        echo "warning: genesis account $address for '$name' already present — skipping duplicate" >&2
    fi
}

init_chain () {
    ensure_jq

    CHAIN_ID="${COSMOS_CHAIN_ID:-$(jq -r '.chain_id' "$CONFIG_JSON")}"
    MONIKER="$(jq -r '.moniker' "$CONFIG_JSON")"
    GENTX_KEY="$(jq -r '.gentx.key' "$CONFIG_JSON")"
    GENTX_AMOUNT="$(jq -r '.gentx.amount' "$CONFIG_JSON")"

    simd init "$MONIKER" --chain-id "$CHAIN_ID" --home "$HOME_DIR" -o

    add_account "$GENTX_KEY" strict

    jq -r '.accounts | keys[]' "$CONFIG_JSON" |
    while read -r name; do
        [ "$name" = "$GENTX_KEY" ] && continue

        add_account "$name"
    done

    simd genesis gentx "$GENTX_KEY" "$GENTX_AMOUNT" --chain-id "$CHAIN_ID" $KEYRING
    simd genesis collect-gentxs --home "$HOME_DIR"

    GENESIS="$CONFIG_FOLDER/genesis.json"
    tmp="$(mktemp)"
    jq '.app_state.gov.params.voting_period="15s"
        | .app_state.gov.params.max_deposit_period="10s"
        | .app_state.gov.params.expedited_voting_period="10s"
        | .app_state.gov.params.min_deposit=[{"denom":"stake","amount":"1"}]
        | .app_state.gov.params.expedited_min_deposit=[{"denom":"stake","amount":"2"}]
        | .app_state.gov.params.quorum="0.000000000000000001"
        | .app_state.gov.params.threshold="0.000000000000000001"' \
        "$GENESIS" > "$tmp" && mv "$tmp" "$GENESIS"
}

if [ ! -d "$CONFIG_FOLDER" ]; then
    init_chain
fi

exec simd start \
    --home "$HOME_DIR" \
    --rpc.laddr tcp://0.0.0.0:26657 \
    --api.enable --api.address tcp://0.0.0.0:1317 --api.enabled-unsafe-cors \
    --grpc.enable --grpc.address 0.0.0.0:9090 \
    --minimum-gas-prices 0.025stake
