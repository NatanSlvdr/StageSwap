#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
workspace_directory=$(CDPATH= cd -- "$script_directory/.." && pwd)
cd "$workspace_directory"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --all-targets
cargo test --release --workspace --all-targets
