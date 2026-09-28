#!/bin/sh
# Build the Cubism Core reference data for tests/cubism_oracle.rs.
#
#   crates/aether-live2d/oracle/run.sh [OUT_DIR]
#
# Downloads, for testing only (nothing here is committed or shipped):
#   * Live2D's sample models from github.com/Live2D/CubismWebSamples;
#   * Live2D Cubism Core for JavaScript, as bundled in the npm package
#     "l2d" (Core 6, reads every MOC3 version).
# Then runs dump.mjs on each model. Needs curl, npm and node.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$root/target/live2d-oracle}
mkdir -p "$out/models"

samples=https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources
for m in Haru Hiyori Mark Natori Rice Mao Wanko Ren; do
    dir="$out/models/$m"
    mkdir -p "$dir"
    [ -f "$dir/$m.moc3" ] || curl -sSfL -o "$dir/$m.moc3" "$samples/$m/$m.moc3"
    [ -f "$dir/$m.model3.json" ] || curl -sSfL -o "$dir/$m.model3.json" "$samples/$m/$m.model3.json"
done

if [ ! -f "$out/core.js" ]; then
    tmp=$(mktemp -d)
    (cd "$tmp" && npm pack l2d@2.1.1 --silent >/dev/null && tar xzf l2d-2.1.1.tgz)
    node "$here/extract-core.mjs" "$tmp/package/dist/index.js" "$out/core.js"
    rm -rf "$tmp"
fi

for m in Haru Hiyori Mark Natori Rice Mao Wanko Ren; do
    node "$here/dump.mjs" "$out/core.js" "$out/models/$m/$m.moc3" "$out/$m" 12
done
echo "wrote $out"
