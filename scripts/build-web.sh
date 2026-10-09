#!/usr/bin/env bash
set -euo pipefail

cargo build --locked --release --target wasm32-unknown-unknown --features web --lib
wasm-bindgen target/wasm32-unknown-unknown/release/scas.wasm \
  --target web --out-dir web/pkg --out-name scas
mkdir -p web/docs
cp -R docs/. web/docs/
