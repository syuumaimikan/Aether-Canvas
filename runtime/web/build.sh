#!/bin/sh
# Build the web player's WebAssembly module into this directory.
#
#   runtime/web/build.sh           build aether_player.wasm
#   runtime/web/build.sh --demo    also export the demo character into model/
#                                  and reference renders into test/reference/
#
# Needs the wasm32 target: rustup target add wasm32-unknown-unknown
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

# Size matters on the web: optimise for it and drop symbol names.
RUSTFLAGS="-C opt-level=s -C strip=symbols" \
    cargo build --release -p aether-player --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/aether_player.wasm "$here/aether_player.wasm"
echo "wrote $here/aether_player.wasm ($(wc -c < "$here/aether_player.wasm") bytes)"

if [ "${1:-}" = "--demo" ]; then
    out=$(mktemp -d)
    cargo run --release -p aether-desktop --example rig_demo -- "$out"
    rm -rf "$here/model" "$here/test/reference"
    cp -r "$out/model" "$here/model"
    cp -r "$out/model-reference" "$here/test/reference"
    rm -rf "$out"
    echo "wrote $here/model and $here/test/reference"
fi
