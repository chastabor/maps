#!/bin/bash -e
cargo build -p maps-wasm --target wasm32-unknown-unknown --profile wasm-release
wasm-bindgen target/wasm32-unknown-unknown/wasm-release/maps_wasm.wasm --target web --out-dir web/pkg
wasm-opt -Os web/pkg/maps_wasm_bg.wasm -o web/pkg/maps_wasm_bg.wasm
# python3 -m http.server -d web
