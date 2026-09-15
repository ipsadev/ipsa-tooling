# `ipsa` — Stellar↔counterparty IBC orchestrator CLI

`ipsa` (binary **`ipsa`**) is the single entry point for the
Stellar↔counterparty IBC v2 bridge. It brings the stack up, builds/pushes images,
deploys the Soroban contracts, uploads the light client, creates clients,
registers counterparties, and reports status — driving **docker**, the
**`stellar`** CLI, and **`stellar-api`** directly. There are no shell scripts.

It lives in its own repo. The stack it drives lives in `ipsa-backend`; point
`STELLAR_IBC_ROOT` at that checkout, or run `ipsa` from inside it.

## How it works

- **Stack-root discovery** — reads `STELLAR_IBC_ROOT`, otherwise walks up from
  the current directory for `docker-compose.yml`. All paths and `docker compose`
  calls are resolved against that root, so the binary works from anywhere once
  the variable is set.
- **Config** — reads `.env` from the stack root (shell env wins, matching the
  rest of the stack). See [Configuration](#configuration).
- **Health probes** — native HTTP (counterparty REST, api `/health`) and TCP (gateway
  gRPC) checks; no external tools required for `check` / `status`.
- **Contract addresses** — read from `.env`, never written. Deployment is done by
  `contracts-cd` in the [ipsa-contracts](https://github.com/ipsadev/ipsa-contracts)
  repo; take the addresses it reports and set them in `.env`. Commands that need
  an address fail with the variable name when it is missing.

---

## Install / run

```sh
export STELLAR_IBC_ROOT=/path/to/ipsa-backend

# run in place (from this repo)
cargo run -- <command>

# install the `ipsa` binary (either of these)
cargo run -- install   # self-install to the cargo bin dir
cargo install --path .

# then, from anywhere:
ipsa <command>
```

Get help for any command or group:

```sh
ipsa --help
ipsa <group> --help
ipsa <group> <command> --help
```

---

## Command overview

| Group | Commands |
|---|---|
| ops | `install` · `check` · `up` · `down` · `start` |
| `cosmos` | `start` · `stop` · `status` · `testnet [--balance <addr>]` |
| `tx clients` | `create [--cosmos\|--stellar] [--force]` · `counterparty [--cosmos\|--stellar]` |
| `tx contracts` | `upload` · `deploy [--stellar\|--cosmos] [--contract <name>] [--force] [--attestation]` · `invoke` |
| `tx transfer` | `--from --to --amount [--denom --timeout-secs --no-mint]` (chain inferred from the addresses) |
| `tx query` | `--clients [--stellar\|--cosmos] [--client-id]` · `--address [--denom]` (balance reads) |
| `services` | `pull` · `up` · `restart` · `down` · `build` · `push` (`[--api\|--gateway\|--cosmos]`) |
| `gateway` | `query` |
| `test` | `[--ics ics02-clients\|ics02-counterparty\|ics20-transfer\|ics20-relay\|ics02-query]` |

---

## Top-level (ops) commands

### `ipsa install`
Installs the `ipsa` binary to the cargo bin dir (`cargo install --path
ipsa --force`) and reports whether that dir is on your `PATH`.

### `ipsa check`
Checks prerequisites and configuration, then probes service health. Reports:
toolchain (`docker`, `stellar`, `cargo`), `.env` presence, key config vars
(`STELLAR_SIGNING_KEY`, `ROUTER_CONTRACT_ADDRESS`, …), the configured endpoints,
the deployed contract ids (from `.env`), and the live state of the counterparty
chain (the `cosmos` compose service, `simd-1`), `stellar-api`, and the gateway
gRPC port. Always exits 0.

### `ipsa up [--cosmos | --stellar]`
Brings the stack up via `docker compose up -d`.

| Flag | Effect |
|---|---|
| _(none)_ | start `cosmos` + `api` + `gateway` |
| `--cosmos` | start only `cosmos` |
| `--stellar` | start only `api` + `gateway` |

### `ipsa down [--volumes]`
Stops the stack via `docker compose down`.

| Flag | Effect |
|---|---|
| `--volumes` | also remove named volumes (wipes chain + relayer-key state) |

### `ipsa start`
Full start: pull images → start `cosmos` → start `api` + `gateway` → deploy
contracts → upload the light-client wasm → import relayer keys → provision the
sender + receiver accounts. Each step is skippable; the chain/service steps are
idempotent (probe first, start if down).

| Flag | Effect |
|---|---|
| `--skip-images` | skip pulling the docker images |
| `--skip-contracts` | skip the Soroban contract deploy |
| `--skip-accounts` | skip provisioning the sender + receiver accounts |
| `--force-redeploy` | redeploy contracts even if `ROUTER_CONTRACT_ADDRESS` is set |

---

## `cosmos` — local counterparty devnet

Lifecycle for the `simd-1` chain (the `cosmos` compose service —
`ghcr.io/cosmos/ibc-go-wasm-simd:v11.0.0`, ibc-go v11 + `08-wasm`).

| Command | Flags | What it does |
|---|---|---|
| `start` | — | `docker compose up -d cosmos` + wait for the first block |
| `stop` | — | `docker compose stop cosmos` |
| `status` | — | probe RPC + print network/endpoints |
| `testnet` | `--balance <addr>` | probe the public counterparty testnet (`provider`) — health + node/app version; with `--balance` read an account's balance |

The two genesis accounts each map to two `.env` vars, set on a fresh checkout
before `start`:

| Account | Mnemonic var | Hex signer var | Used by |
|---|---|---|---|
| validator | `COSMOS_VALIDATOR_MNEMONIC` | `COSMOS_VALIDATOR_PRIVATE_KEY` | genesis validator; funds accounts and casts the gov **vote** (carries stake) |
| relayer | `COSMOS_RELAYER_MNEMONIC` | `COSMOS_RELAYER_PRIVATE_KEY` | relayer key; submits the gov store-code **proposal** |

`docker-compose.yml` passes the mnemonics into the counterparty container and
`setup.sh` recovers + funds those accounts at genesis; the api signs gov
messages (store-code proposal during `upload-wasm`) with the hex keys. The hex
key is just the mnemonic's private key, so each account's two vars must stay in
sync.

---

## `tx` — write operations

Mutating commands live under `tx`. For `clients` and `contracts`, a `--cosmos` /
`--stellar` flag selects the side; **omit it to act on both**.

### `ipsa tx clients create [--cosmos | --stellar] [--force]`
Create the counterparty (`07-tendermint`, on Stellar) and/or Stellar
(`08-wasm`, on the counterparty) client; with neither flag, creates both. `--force` recreates even if the
id is already set. The Stellar client needs the light-client wasm stored on the
counterparty first (`tx contracts deploy --cosmos`).

### `ipsa tx clients counterparty [--cosmos | --stellar]`
Register the counterparty on the chosen side (IBC v2 `registerCounterparty`, no
handshake); with neither flag, registers on both. The Stellar side goes through
the gateway prepare→sign→submit path, the counterparty side through ibc-go.

### `ipsa tx contracts upload`
Build and upload (install) every Soroban contract wasm to the network, printing
each wasm hash.

### `ipsa tx contracts deploy [--stellar | --cosmos] [--contract <name>] [--force] [--attestation]`
- `--stellar` (default): deploy the Soroban contracts. With `--contract <name>`
  (the wasm file name without `.wasm`, e.g. `stellar_ibc_router`) deploy just
  that one; omit it for the full build + deploy + wire-router orchestration
  (`--force` redeploys, `--attestation` also deploys the attestation LC). The
  addresses are printed, not written; set them in `.env` yourself.
- `--cosmos`: store the light-client code on the counterparty chain (08-wasm gov store-code).

### `ipsa tx contracts invoke --id <contract> -- <fn> <args>`
Invoke a function on a deployed Soroban contract; the function name and its args
pass through verbatim after `--`.

```sh
ipsa tx contracts invoke --id CB2L... -- register_port --port_id transfer --app_address CASB...
```

### `ipsa tx transfer --from <addr> --to <addr> --amount <n> [--denom --timeout-secs --no-mint --relay --wait]`
Originate an ICS-20 transfer. The source/destination chain is inferred from each
address (`cosmos1…` → counterparty, `G…`/`C…` → Stellar) and routed
accordingly — Stellar→counterparty is wired, counterparty→Stellar is pending
(M4). `--denom` defaults to
`stake`.

The transaction hash is printed on stdout (and nothing else is), so it can be
captured: `HASH=$(ipsa tx transfer … | tail -1)`. Nothing relays that
transaction on its own — pass `--relay` to hand it straight to the relayer, or
run `tx relay --tx <hash>` yourself afterwards.

### `ipsa tx relay --tx <hash> [--stellar | --cosmos] [--chain-id <id>] [--status --wait]`
Hand a source transaction to the `ibc-relayer`. IBC v2 relaying is
request-driven — nothing scans the chains — so a transfer is only relayed once
this runs. Direction follows the source chain: a Stellar tx relays
Stellar→counterparty, a counterparty tx relays counterparty→Stellar. The flag
is optional (the
chain is read off the hash case, and an ambiguous hash asks for the flag).

`--status` reports the packets without submitting, `--wait` polls until each one
completes or fails (`--timeout-secs`, `--interval-secs`). The call goes through
`stellar-api` (`POST /relay`, `GET /relay/status`), which owns the gRPC
connection to the relayer.

```sh
ipsa tx relay --tx <stellar-tx-hash> --stellar --wait
ipsa tx relay --tx <counterparty-tx-hash> --cosmos
ipsa tx relay --tx <hash> --status
```

---

## `tx query` — read client states + balances

```sh
ipsa tx query --clients [--stellar | --cosmos] [--client-id <id>]
ipsa tx query --address <addr> [--denom <denom>]
```

`--clients` reads client states: with `--stellar` / `--cosmos` it scopes to one
network (Stellar via the api `/stellar/clients`, the counterparty via the IBC REST
`client_states`), with neither it reads both, and `--client-id` restricts to a
single client. `--address` reads an account's balances (the chain is inferred
from the address); `--denom` filters to a single denom.

---

## `test` — ICS integration flows

```sh
ipsa test [--ics ics02-clients|ics02-counterparty|ics20-transfer|ics20-relay|ics02-query]
```

Runs the happy-path integration flow for each ICS milestone against a **running
stack** (bring it up first with `ipsa start`). With no `--ics`, every
flow runs in dependency order and a summary is printed; the command exits
non-zero if any flow fails. Flows are labelled by IBC standard — client
lifecycle, counterparty registration, and client-state queries are all ICS-02
under Eureka; token transfer is ICS-20.

| ICS | What it asserts |
|---|---|
| `ics02-clients` | creates the counterparty (`07-tendermint`) and Stellar (`08-wasm`) clients and checks the returned ids |
| `ics02-counterparty` | bootstraps clients + counterparties and checks the Stellar router lists the paired clients |
| `ics20-transfer` | originates a Stellar→counterparty ICS-20 transfer, waits for the relay round trip to close, and checks the counterparty voucher increased |
| `ics20-relay` | relays a transfer each way — Stellar→counterparty in `XLM`, counterparty→Stellar in `stake` — and checks both are delivered and acknowledged |
| `ics02-query` | checks the api `/health` + `/stellar/clients` reads and the counterparty `client_states` read |

The flow bodies live in `src/tests/` (one file per ICS). CI runs them via the
`ics-flows.yml` workflow — each ICS is a separate, independently re-runnable
step.

---

## `services` — service lifecycle + images

```sh
ipsa services <pull|up|restart|down|build|push> [--api|--gateway|--cosmos]
```

One namespace for the `api`, `gateway`, and `cosmos` services. A
`--api` / `--gateway` / `--cosmos` flag selects the target;
**omitting it acts on all**. Image tags resolve from `.env`.

| Command | What it does |
|---|---|
| `pull` | `docker compose pull` the selected image(s) |
| `up` | pull then `docker compose up -d` |
| `restart` | remove the existing container(s) (`rm -s -f`) then `up` |
| `down` | stop + remove the container(s) (`rm -s -f`) |
| `build` | `docker build` the image(s) for the host arch |
| `push` | `docker build` + `docker push` for the host arch |

`build` / `push` cover the custom images only — `--cosmos` is an upstream image
and is rejected for build/push. The **ibc-relayer** is built from a separate
repo: the CLI clones `RELAYER_REPO_URL` into `target/ibc-relayer` (or updates
it), checks out `RELAYER_BRANCH`, then builds from there.

Both build for the **host architecture only** — multi-arch (amd64 + arm64)
manifests are published by the CD workflows (`api-cd` / `gateway-cd`), which
build each arch on a native runner. Building amd64 locally on an arm64 host
would cross-compile under QEMU, so it is intentionally left to CI.

---

## `gateway` — gateway service

| Command | What it does |
|---|---|
| `query` | direct gateway gRPC reads — *pending* |

Container lifecycle lives under `services`.

---

## Typical workflows

First run from a clean machine:

```sh
cargo install --path ipsa   # install the ipsa binary
ipsa check                # docker/stellar/cargo present? .env filled? everything green?
ipsa start                # images, chains, contracts, wasm, keys, accounts

ipsa tx clients create    # create both clients (counterparty on Stellar, Stellar on the counterparty)
ipsa tx query --clients   # read client states on both networks
```

Day-to-day:

```sh
ipsa up                              # bring the stack up
ipsa services restart --api          # pull latest + recreate just the api
ipsa tx contracts deploy --stellar --force   # redeploy contracts, print addresses
ipsa services restart --gateway      # recreate the gateway to pick up the new ROUTER_CONTRACT_ADDRESS
ipsa down                            # stop the stack
```

---

## Configuration

Read from `stellar-ibc/.env` (shell env overrides). Defaults shown.

| Variable | Default | Used by |
|---|---|---|
| `STELLAR_IBC_ROOT` | _(auto-discovered)_ | repo-root override |
| `COSMOS_CHAIN_ID` | `simd-1` | status, clients, keys |
| `COSMOS_REST_URL` | `http://127.0.0.1:1317` | check/status/start probes |
| `COSMOS_RPC_URL` | `http://127.0.0.1:26657` | clients (RPC probe) |
| `STELLAR_API_URL` | `http://127.0.0.1:8101` | status, clients list, upload-wasm |
| `STELLAR_GATEWAY_GRPC_PORT` | `50052` | gateway gRPC probe |
| `STELLAR_SIGNING_KEY` | _(required)_ | deploy + the stellar relayer key |
| `STELLAR_RPC_URL` | `https://soroban-testnet.stellar.org` | contracts (stellar CLI) |
| `NETWORK_PASSPHRASE` | `Test SDF Network ; September 2015` | contracts (stellar CLI) |
| `DEPLOYER_IDENTITY` | `admin` | contracts deploy/invoke `--source` |
| `ROUTER_CONTRACT_ADDRESS` / `TRANSFER_CONTRACT_ADDRESS` / `DEPLOYER_ADDRESS` | _(set by deploy-all)_ | status, deploy-all idempotency |
| `COSMOS_CLIENT_ID` / `STELLAR_CLIENT_ID` | _(set by clients cmds)_ | clients idempotency |
| `TRANSFER_PORT` / `MOCK_CLIENT_TYPE` / `ATTESTATION_CLIENT_TYPE` / `TENDERMINT_CLIENT_TYPE` | `transfer` / `mock` / `attestation` / `07-tendermint` | router wiring |
| `API_IMAGE` / `API_TAG` / `API_REGISTRY` | `amandagonsalvesx/stellar-eureka-api` / `latest` / _(none)_ | api image to pull/run |
| `GATEWAY_IMAGE` / `GATEWAY_TAG` / `GATEWAY_REGISTRY` | `amandagonsalvesx/stellar-eureka-gateway` / `latest` / _(none)_ | gateway image to pull/run |

> `RELAYER_REPO_URL` / `RELAYER_BRANCH` locate the relayer source for
> `services build/push --relayer` (cloned into `target/ibc-relayer`);
> `DOCKER_USERNAME` / `DOCKER_TOKEN` authenticate the push.

---

## Source layout

```
ipsa/src/
  main.rs            clap command tree + dispatch
  config.rs          base Config: cosmos · stellar · relayer · api · gateway · deployment
  repo.rs            stack-root discovery
  run.rs             process helpers (command / capture / compose / piped)
  tools.rs           typed wrappers over run (stellar · gaiad · docker)
  probe.rs           http / tcp health probes
  logger.rs          TTY-aware status logger
  shared.rs          chain_of · print_clients · env_upsert · check helpers
  install.rs         install command (self-install to the cargo bin dir)
  check.rs           prerequisites + config + service-health check
  stack.rs           up / down (docker compose)
  start.rs           full bring-up orchestration
  service.rs         shared docker-compose start wrapper
  accounts/          sender + receiver account provisioning (stellar + counterparty)
  tx/                write operations grouped under `tx`
    mod.rs           tx command tree (clients · contracts · transfer · query) + side routing
    clients/         create (cosmos/stellar) · counterparty · config
    contracts/       upload · deploy_all · wasm · build · config (+ deploy_one/invoke)
    transfer/        ICS-20 transfer origination (stellar → counterparty)
    query/           client-state reads (clients.rs) + address-routed balances (balances.rs)
  services/          service lifecycle + images grouped under `services`
    mod.rs           pull/up/restart/down/build/push (api · gateway · cosmos · relayer)
    cosmos/          counterparty (simd-1) chain config + lifecycle (start/stop/status/testnet)
    stellar/         stellar chain config
    gateway/         gRPC query · config
  tests/             ICS integration flows (ipsa test)
proto/               gateway + proof-api service definitions, copied from
                     ipsa-backend; `just protos` proves they have not drifted
scripts/             one-off operator scripts
```

## just

`just ci` runs what CI runs: `fmt-check`, `lint`, `test`, `audit`. `just protos`
diffs `proto/` against an `ipsa-backend` checkout.

Image build + push is `ipsa services build` / `services push`, which resolve
image refs from `.env` and clone the relayer source per `RELAYER_REPO_URL` /
`RELAYER_BRANCH`.
