# `ipsa` — Stellar↔counterparty IBC orchestrator CLI

`ipsa` (binary **`ipsa`**) is the single entry point for the
Stellar↔counterparty IBC v2 bridge. It brings the stack up, builds/pushes images,
deploys the Soroban contracts, uploads the light client, creates clients,
registers counterparties, and reports status — driving **docker**, the
**`stellar`** CLI, and **`ipsa-api`** directly. There are no shell scripts.

It lives in its own repo and needs no other checkout. The stack it drives ships
in [`stack/`](stack): the compose file, the simd devnet assets, the stellar-core
watcher config and the relayer config template. The api, gateway and relayer run
as published images; where the contracts and light-client releases, the images
and the relayer and verifier sources live is read from `.env`, never built in.

## How it works

- **The stack** — `stack/` beside this README, found from wherever `ipsa` runs
  because the binary records the folder it was built from (`IPSA_STACK_DIR`
  points it at a copy instead). All paths and `docker compose` calls resolve
  against it, and the compose project is named `ipsa`. On first use `ipsa`
  copies `stack/relayer.example.yml` to `stack/relayer.yml`, which it then edits
  as it deploys contracts and creates clients; that copy, `relayer-keys.json`,
  `dist/` and the store-code proposals stay untracked.
- **Config** — one file, `.env` beside this README. `ipsa` loads it and every
  `docker compose` it runs inherits it. See [Configuration](#configuration).
- **Health probes** — native HTTP (counterparty REST, api `/health`) and TCP (gateway
  gRPC) checks; no external tools required for `check` / `status`.
- **Contracts** — the Soroban wasm comes from the GitHub release named by
  `CONTRACTS_REPOSITORY` (owner/name) and `CONTRACTS_VERSION` (the tag). It is downloaded with `gh` into
  `dist/contracts/<tag>/` and every file is checked against the release's
  `SHA256SUMS` before anything is uploaded or deployed; a verified copy is reused.
  Run `gh auth login` first if the repository is private. Nothing is built locally.
  `tx contracts deploy --stellar` writes the addresses it deploys, and the
  router's genesis ledger, into `.env`. Commands that need an address fail with
  the variable name when it is missing.

---

## Install / run

```sh
cp .env.example .env   # then fill it in

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
chain (the `cosmos` compose service, `simd-1`), `ipsa-api`, and the gateway
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
Full start, from an empty `.env` to a corridor the relayer can carry both ways:

1. pull images
2. start `cosmos` and the stellar-core watcher
3. start `api` + `gateway`
4. deploy the Soroban contracts and write their addresses and
   `ROUTER_GENESIS_LEDGER` to `.env`, then recreate `api` + `gateway` so they
   read them
5. provision the sender + receiver accounts
6. store the light-client wasm on cosmos (gov store-code + vote)
7. create the `07-tendermint` client on Stellar and the `08-wasm` client on
   cosmos, routing both in `relayer.yml`
8. register each client as the other's counterparty
9. pair the Stellar ↔ EVM corridor: when the two clients named by
   `STELLAR_EVM_CLIENT_ID` and `EVM_STELLAR_CLIENT_ID` do not already name each
   other on the Stellar router and the EVM router, create the `sepolia` client on
   the Stellar router and an `SP1ICSStellar` client on the EVM router (through the
   proof API), register both counterparties, write the two ids to `.env` and
   route them in `relayer.yml`. The EVM client id is `EVM_STELLAR_CLIENT_ID`, or
   the next free `<prefix>-N` when that one is taken. Needs `stellar-proof-api`
   for this router at `PROOF_API_URL` (`ipsa start` starts it when nothing
   listens there) and `EVM_PRIVATE_KEY`, an account holding the EVM router's
   `addClient` role; without the key `ipsa start` stops, unless `--skip-evm`
10. import the relayer keys, start postgres, and start the relayer, recreating
   it when `relayer.yml` changed

Every step is idempotent: a deployed router, a stored checksum and existing
client ids are kept. Counterparties are only registered when a client was
created in this run. When the contracts are redeployed, the clients are
recreated on the new router.

| Flag | Effect |
|---|---|
| `--skip-images` | skip pulling the docker images |
| `--skip-accounts` | skip provisioning the sender + receiver accounts |
| `--skip-clients` | skip the light-client wasm, the clients and the counterparties (both corridors) |
| `--skip-evm` | skip only the Stellar ↔ EVM clients and counterparties |
| `--skip-relayer` | skip the relayer keys, postgres and the relayer |
| `--deploy-contracts` | redeploy the contracts even if `ROUTER_CONTRACT_ADDRESS` is set |

---

## `proof-api` — stellar-proof-api on the host

`stellar-proof-api` proves the Stellar ↔ EVM corridor and forwards the Cosmos pairs
to the gateway. It runs on the host, not in the stack, from its own checkout
(`ibc-contracts`), which `PROOF_API_DIR` names.

| Command | What it does |
|---|---|
| `start` | runs `PROOF_API_BINARY start --config PROOF_API_CONFIG` in `PROOF_API_DIR`, with the variables in that checkout's `PROOF_API_ENV_FILE` (the SP1 prover keys), detached; waits until it listens on `PROOF_API_URL`. Does nothing when something already listens there |
| `stop` | stops whatever listens on `PROOF_API_URL`, and the pid in `stack/proof-api.pid`; refuses when that process is not `stellar-proof-api`; SIGTERM, then SIGKILL after 30s |
| `restart` | `stop`, then `start` — the way to make it reread its config |
| `status` | whether it listens, and its pid |

Output goes to `stack/proof-api.log`. A process started by hand is found by its
port, so `restart` takes it over. `ipsa start` runs `start` before the EVM step
when nothing is listening.

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
  addresses and `ROUTER_GENESIS_LEDGER` are written to `.env`; restart the api
  and gateway to pick them up.
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
`ipsa-api` (`POST /relay`, `GET /relay/status`), which owns the gRPC
connection to the relayer.

```sh
ipsa tx relay --tx <stellar-tx-hash> --stellar --wait
ipsa tx relay --tx <counterparty-tx-hash> --cosmos
ipsa tx relay --tx <hash> --status
```

---

## The Stellar ↔ EVM corridor

Stellar and an EVM chain are joined by two zk light clients: an `SP1ICSStellar`
client on the EVM chain, and an SP1 Ethereum client on the Soroban router. Packets
are delivered the same way as on the Cosmos corridor — `ipsa` hands the source
transaction to `ipsa-api`, and `ibc-relayer` asks the proof API for the
transactions, submits them, and carries the acknowledgement back. The proof API is
`stellar-proof-api` from the `ibc-contracts` fork; the relayer reaches it as its
`ibcv2_proof_api` (see `stack/relayer.example.yml`).

### `ipsa tx transfer --from G… --to 0x… --amount <n> [--denom XLM]`

A `0x…` destination routes the transfer to the corridor's Soroban transfer app
(`STELLAR_EVM_TRANSFER`) on the client `STELLAR_EVM_CLIENT_ID`. `--denom XLM`
resolves to the native asset's contract id, which is the denom the app escrows
under; any other denom is passed through as the app knows it. The signing identity
is `EVM_STELLAR_SENDER_IDENTITY`, or `DEPLOYER_IDENTITY` when that is unset, and it
must own the `--from` address.

The timeout is raised to at least six hours: a packet cannot be received until
the history archive publishes its ledger's checkpoint and two proofs are made.
`--relay` hands it to the relayer straight away, and `--wait` follows it.

### `ipsa tx transfer --from 0x… --to G… --amount <n> --denom <0x token>`

A `0x…` source sends from the EVM side: `approve` the amount to
`EVM_ICS20_TRANSFER`, then `ICS20Transfer.sendTransfer` on the client
`EVM_STELLAR_CLIENT_ID`, each simulated before it is sent from `EVM_PRIVATE_KEY`
(which must own `--from`). `--denom` is the token contract; an `IBCERC20`
voucher's address sends that asset home. The timeout is held between six hours
and the router's one-day ceiling.

### `ipsa tx relay --tx <stellar hash> --evm` and `ipsa tx relay --tx <0x hash>`

Hands the transaction to the relayer under the corridor's chain ids —
`STELLAR_EVM_CHAIN_ID` for a Stellar source, `EVM_CHAIN_ID` for an `0x…` one — and,
with `--wait`, follows its packets until the acknowledgement is back. The wait
defaults to an hour: an EVM source waits for Ethereum finality, a Stellar source
for its history-archive checkpoint, and each leg is proved on the Succinct network.

### `ipsa tx relay … --direct`

Skips the relayer: `ipsa` calls the proof API's `RelayByTx` itself (at
`PROOF_API_URL`, under the same chain ids) and submits what it returns.

- A Stellar source (`--evm`) returns `ICS26Router.multicall(updateClient,
  recvPacket | ackPacket)` calldata and an address. `ipsa` refuses to send it
  anywhere but `EVM_ICS26_ROUTER`, simulates it, sends it from `EVM_PRIVATE_KEY`,
  and waits for the receipt.
- An `0x…` source returns Soroban calls on `STELLAR_EVM_ROUTER` — `update_client`
  for each header, then `recv_packet` or `acknowledge_packet`. `ipsa` refuses any
  call to another contract or any other function, then submits each with the
  stellar CLI (`tx update sequence-number next` → `tx simulate` → `tx sign` →
  `tx send`), signed by `STELLAR_EVM_RELAYER_IDENTITY` or `DEPLOYER_IDENTITY`.

The resulting transaction hashes are printed on stdout. An acknowledgement is
relayed the same way: pass the EVM transaction that received a Stellar packet, or
`--evm` with the Stellar transaction that received an EVM packet. When the proof
API runs with `"prover": "execute"` it proves nothing and returns an empty
transaction, which `ipsa` reports as a passed dry run. The proof API reports
failures to callers only as a generic status; the reason is in its own log.

### `ipsa tx clients create --evm` and `ipsa tx clients counterparty --evm`

`create --evm` creates both zk light clients through the proof API's
`CreateClient`. On Stellar it submits the one `create_client` call the proof API
returns — the genesis carries the next sync committee, so the client can cross
periods — and reads the new id from the transaction's return value. On the EVM
side it deploys the `SP1ICSStellar` creation code the proof API returns (genesis at
the latest archived ledger, the router as role manager) and registers it with
`addClient` under `EVM_STELLAR_CLIENT_ID`, naming the Stellar client as its
counterparty. `counterparty --evm` then registers `EVM_STELLAR_CLIENT_ID` as the
Soroban client's counterparty.

Pick a fresh `EVM_STELLAR_CLIENT_ID` whenever the Stellar side is a new router: its
sequences restart at 1, and the EVM router still holds receipts for the old ones
under an old id.

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `EVM_RPC_URL` | _(required)_ | JSON-RPC endpoint of the EVM chain |
| `EVM_PRIVATE_KEY` | _(required)_ | key that sends EVM transfers, and relay transactions with `--direct` (then it needs the router's `RELAYER_ROLE`, as the relayer's own key does) |
| `EVM_ICS26_ROUTER` | _(required)_ | `ICS26Router` proxy; the only address `ipsa` will send a relay to |
| `EVM_STELLAR_CLIENT_ID` | `stellar-testnet-1` | the Stellar client's id on the EVM router |
| `EVM_ICS20_TRANSFER` | _(required for EVM transfers)_ | `ICS20Transfer` proxy |
| `STELLAR_EVM_ROUTER` | `ROUTER_CONTRACT_ADDRESS` | the corridor's Soroban router; the only contract `ipsa` will call for a relay |
| `STELLAR_EVM_TRANSFER` | `TRANSFER_CONTRACT_ADDRESS` | the corridor's Soroban transfer app |
| `STELLAR_EVM_RELAYER_IDENTITY` | `DEPLOYER_IDENTITY` | stellar CLI identity that signs Soroban relay transactions |
| `STELLAR_EVM_CLIENT_ID` | `sepolia-0` | the EVM client's id on the Soroban router |
| `STELLAR_EVM_CLIENT_TYPE` | `sepolia` | the client type `create --evm` asks the Soroban router for |
| `EVM_STELLAR_SENDER_IDENTITY` | `DEPLOYER_IDENTITY` | stellar CLI identity that signs transfers |
| `STELLAR_EVM_CHAIN_ID` / `EVM_CHAIN_ID` | `STELLAR_CHAIN_ID` / `11155111` | the corridor's chain ids: the relayer's `chain_id`s and the proof API modules' `src_chain` / `dst_chain` |
| `PROOF_API_URL` | `http://127.0.0.1:3100` | where `stellar-proof-api` listens (`--direct` and `clients create --evm`) |

One Stellar router serves both corridors, so the Stellar side follows the stack's own
`ROUTER_CONTRACT_ADDRESS`, `TRANSFER_CONTRACT_ADDRESS` and `STELLAR_CHAIN_ID` unless the
`STELLAR_EVM_*` variables say otherwise. [`.env.example`](.env.example) lists the EVM values
for the live Sepolia corridor.

---

## The Stellar ↔ Cosmos Hub testnet corridor

`COSMOS_NETWORK=testnet` in `.env` points the Cosmos side at the Cosmos Hub testnet
(`provider`) instead of the local `simd-1`; `devnet`, the default, keeps the local chain.
The `COSMOS_TESTNET_*` variables describe `provider`, and `COSMOS_RELAYER_PRIVATE_KEY`
signs on both networks, so it must hold ATOM for testnet use.

What changes with `testnet`:

- **Clients and counterparties** are created and registered on `provider`, and their ids
  are recorded as `COSMOS_TESTNET_CLIENT_ID` (the 07-tendermint client on Stellar) and
  `COSMOS_TESTNET_STELLAR_CLIENT_ID` (the 08-wasm client on `provider`), so switching back
  to `devnet` finds the `simd-1` ids untouched. The 08-wasm client needs the light-client
  code already stored on `provider` by governance (`ipsa tx contracts upload-wasm --testnet`).
- **Transfers from Cosmos** are built and signed here as a `MsgTransfer` with
  `COSMOS_RELAYER_PRIVATE_KEY`, instead of running `simd` in the devnet container, so
  `--from` must be that key's address. Transfers from Stellar are unchanged.
- **The stack** gets `provider`'s chain id, client ids and endpoints: `ipsa` exports them
  before running compose, so the gateway stamps `provider` into the messages it builds.
- **The relayer** relays through the `provider` block in `relayer.yml`, which uses the
  public RPC and gRPC (TLS) and pays `uatom` at a gas price of 0.005, twice the chain's
  minimum. `ipsa relayer keys-import` writes the Cosmos key under both `simd-1` and
  `provider`.
- **The proof API** is started with its config inline (`PROOF_API_CONFIG_JSON`) and its
  `forward` modules pointed at the active Cosmos chain, so the same `testnet.json` serves
  both networks. This needs a `stellar-proof-api` built with inline config support.

```sh
ipsa tx clients create --cosmos       # 07-tendermint client tracking provider, on Stellar
ipsa tx clients create --stellar      # 08-wasm client tracking Stellar, on provider
ipsa tx clients counterparty
ipsa tx transfer --from G… --to cosmos1… --amount 1 --relay --wait
ipsa tx transfer --from cosmos1… --to G… --amount 1000 --denom uatom --relay --wait
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
**omitting it acts on all**. Image tags resolve from `.env`. `build` and `push`
apply to the relayer and verifier only: the api and gateway images are
`ipsa-backend`'s releases, and the stack only pulls them.

| Command | What it does |
|---|---|
| `pull` | `docker compose pull` the selected image(s) |
| `up` | pull then `docker compose up -d` |
| `restart` | remove the existing container(s) (`rm -s -f`) then `up` |
| `down` | stop + remove the container(s) (`rm -s -f`) |
| `build` | `docker build` the image(s) for the host arch |
| `push` | `docker build` + `docker push` for the host arch |

`build` / `push` cover the custom images only — `--cosmos` is an upstream image
and is rejected for build/push. The **relayer** is built from a separate
repo: the CLI clones `RELAYER_REPO_URL` into `target/relayer` (or updates
it), checks out `RELAYER_BRANCH`, then builds from there; with no
`RELAYER_REPO_URL` it builds the local checkout at `RELAYER_CONTEXT`. The
**verifier** is cloned the same way from `VERIFIER_REPO_URL` into `target/verifier`.

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

`ipsa` reads one file, `.env` beside this README, and values in it override the
shell. It is found from wherever `ipsa` runs because the binary records the folder
it was built from. It holds everything: release locations, test accounts, image
build sources, the EVM corridor, and what the stack's services read — contract
addresses, ports, chain and client ids, the images it runs. Every `docker compose`
call inherits it. [`.env.example`](.env.example) lists all of it; copy it to `.env`.

`ipsa` writes what it creates back to the same file: contract addresses, the
router's genesis ledger, client ids, account keys and the stored light-client
checksum. Defaults shown.

| Variable | Default | Used by |
|---|---|---|
| `IPSA_STACK_DIR` | `stack/` beside this README | run a copy of the stack from elsewhere |
| `COSMOS_CHAIN_ID` | `simd-1` | status, clients, keys |
| `COSMOS_REST_URL` | `http://127.0.0.1:1317` | check/status/start probes |
| `COSMOS_RPC_URL` | `http://127.0.0.1:26657` | clients (RPC probe) |
| `STELLAR_API_URL` | `http://127.0.0.1:8101` | status, clients list, upload-wasm |
| `STELLAR_GATEWAY_GRPC_PORT` | `50052` | gateway gRPC probe |
| `STELLAR_SIGNING_KEY` | _(required)_ | deploy + the stellar relayer key |
| `STELLAR_RPC_URL` | `https://soroban-testnet.stellar.org` | contracts (stellar CLI) |
| `NETWORK_PASSPHRASE` | `Test SDF Network ; September 2015` | contracts (stellar CLI) |
| `DEPLOYER_IDENTITY` | `admin` | contracts deploy/invoke `--source` |
| `CONTRACTS_REPOSITORY` / `CONTRACTS_VERSION` | _(required to deploy)_ | the GitHub release (owner/name, tag) `tx contracts deploy --stellar` and `upload` download |
| `LIGHT_CLIENT_REPOSITORY` / `LIGHT_CLIENT_VERSION` | _(required unless `LIGHT_CLIENT_WASM_DIR` is set)_ | the light-client release (owner/name, tag) `start` and `tx contracts deploy --cosmos` download and store on cosmos |
| `LIGHT_CLIENT_WASM_DIR` | _(unset)_ | a local light-client build to store instead of downloading a release; must hold `stellar_light_client.wasm`, its `.gz` and `checksums.txt` |
| `ROUTER_CONTRACT_ADDRESS` / `TRANSFER_CONTRACT_ADDRESS` / `DEPLOYER_ADDRESS` | _(set by deploy-all)_ | status, deploy-all idempotency |
| `COSMOS_CLIENT_ID` / `STELLAR_CLIENT_ID` | _(set by clients cmds)_ | clients idempotency |
| `TRANSFER_PORT` / `MOCK_CLIENT_TYPE` / `ATTESTATION_CLIENT_TYPE` / `TENDERMINT_CLIENT_TYPE` | `transfer` / `mock` / `attestation` / `07-tendermint` | router wiring |
| `API_IMAGE` / `API_TAG` / `API_REGISTRY` | _(required)_ / `latest` / _(none)_ | api image the stack pulls |
| `GATEWAY_IMAGE` / `GATEWAY_TAG` / `GATEWAY_REGISTRY` | _(required)_ / `latest` / _(none)_ | gateway image the stack pulls |
| `RELAYER_IMAGE` / `VERIFIER_IMAGE` | _(required to build)_ | relayer and verifier images to build/push |
| `RELAYER_CONTEXT` | _(none)_ | a local relayer checkout to build when `RELAYER_REPO_URL` is unset |

> `RELAYER_REPO_URL` / `RELAYER_BRANCH` locate the relayer source for
> `services build/push --relayer` (cloned into `target/relayer`);
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
    contracts/       upload · deploy_all · wasm · release · config (+ deploy_one/invoke)
    transfer/        ICS-20 transfer origination (stellar → counterparty)
    evm/             the Stellar ↔ EVM corridor: config · transfers both ways · direct relays through the proof API · client creation · Soroban host-function submission
    query/           client-state reads (clients.rs) + address-routed balances (balances.rs)
  services/          service lifecycle + images grouped under `services`
    mod.rs           pull/up/restart/down/build/push (api · gateway · cosmos · relayer)
    cosmos/          counterparty (simd-1) chain config + lifecycle (start/stop/status/testnet)
    stellar/         stellar chain config
    gateway/         gRPC query · config
  tests/             ICS integration flows (ipsa test)
proto/               gateway + proof-api service definitions, copied from
                     the gateway; `just protos <stack>` proves they have not drifted
scripts/             one-off operator scripts
```

## just

`just ci` runs what CI runs: `fmt-check`, `lint`, `test`, `audit`. `just protos <stack>`
diffs `proto/` against a stack checkout.

Image build + push is `ipsa services build` / `services push`, which resolve
image refs from `.env` and clone the relayer source per `RELAYER_REPO_URL` /
`RELAYER_BRANCH`.

## License

Copyright 2026 Amanda Gonsalves. Licensed under the [Apache License 2.0](LICENSE).
