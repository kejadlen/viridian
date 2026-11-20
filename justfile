default: all

fmt:
    cargo fmt --all

check:
    cargo check --workspace

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

all: fmt clippy test
