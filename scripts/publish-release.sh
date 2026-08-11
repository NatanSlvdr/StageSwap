#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$script_directory/.."

if ! cargo xwin --version >/dev/null 2>&1; then
    echo "cargo-xwin is missing; install it with: cargo install --locked cargo-xwin" >&2
    exit 1
fi

resource_compiler=$(command -v llvm-rc || true)
if [ -z "$resource_compiler" ] && command -v brew >/dev/null 2>&1; then
    for llvm_formula in llvm llvm@20 llvm@19; do
        llvm_prefix=$(brew --prefix "$llvm_formula" 2>/dev/null || true)
        if [ -x "$llvm_prefix/bin/llvm-rc" ]; then
            resource_compiler="$llvm_prefix/bin/llvm-rc"
            break
        fi
    done
fi
if [ -z "$resource_compiler" ]; then
    echo "llvm-rc is missing; install LLVM with: brew install llvm" >&2
    exit 1
fi

export STAGESWAP_USE_CARGO_XWIN=1
export STAGESWAP_CROSS_COMPILE_RESOURCES=1
export STAGESWAP_WINDOWS_SDK_VERSION=10.0.22621.0
export XWIN_ARCH=x86_64
export XWIN_SDK_VERSION=10.0.22621
export RC_PATH="$resource_compiler"

cargo run --release -p xtask -- publish-release
