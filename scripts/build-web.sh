#!/usr/bin/env bash
set -euo pipefail

cargo build --locked --profile web-release --target wasm32-unknown-unknown --features web --lib
wasm-bindgen target/wasm32-unknown-unknown/web-release/scas.wasm \
  --target web --out-dir web/pkg --out-name scas
rm -rf web/docs
mkdir -p web/docs
cp -R docs/. web/docs/
touch web/.nojekyll
python3 scripts/check-web.py
