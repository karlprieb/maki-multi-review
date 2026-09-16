default:
    @just --list

check:
    cargo check --tests

lint:
    cargo clippy --tests -- -D warnings

test:
    cargo nextest run

fmt:
    cargo fmt --all

fmt-lua:
    stylua plugin/

fmt-check:
    cargo fmt --all -- --check
    stylua --check plugin/
