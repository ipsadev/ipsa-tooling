# orchestrator CLI for the Stellar and Cosmos IBC bridge

lints := "-D warnings -A clippy::manual_is_multiple_of -A clippy::too_many_arguments -A clippy::result_large_err"

default:
    @just --list

# everything CI runs, in CI's order
ci: fmt-check lint test audit

fmt:
    cargo +nightly fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --all-targets -- {{lints}}

test:
    cargo test --locked

# proto/ is copied from ipsa-backend; run locally to prove it has not drifted
protos backend="../ipsa-backend":
    diff -u {{backend}}/crates/gateway/proto/stellar_gateway.proto proto/stellar_gateway.proto
    diff -u {{backend}}/crates/gateway/proto/proofapi.proto proto/proofapi.proto
    @echo "protos match {{backend}}"

audit:
    cargo audit --file Cargo.lock

clean:
    cargo clean
