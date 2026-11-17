#!/bin/sh
# Builds crates/wasm to WebAssembly and generates its JavaScript glue and
# TypeScript types into client/src/wasm/ (not committed: rebuilt from Rust).
#
# Needs, once:  cargo install wasm-bindgen-cli --version <the version below>
# The wasm32 target is installed by rustup from rust-toolchain.toml.
set -eu
cd "$(dirname "$0")/.."

# The command-line tool must match the wasm-bindgen library exactly.
want=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
have=$(wasm-bindgen --version 2>/dev/null | cut -d' ' -f2 || true)
if [ "$have" != "$want" ]; then
  echo "error: wasm-bindgen $want is needed (found: ${have:-none})." >&2
  echo "       Install it with: cargo install wasm-bindgen-cli --version $want" >&2
  exit 1
fi

target_dir=${CARGO_TARGET_DIR:-target}
cargo build -p holotable-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir client/src/wasm \
  "$target_dir/wasm32-unknown-unknown/release/holotable_wasm.wasm"
