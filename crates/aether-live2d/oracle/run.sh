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
    # Everything else the model refers to (textures, motions, physics...).
    for f in $(node "$here/files.mjs" "$dir/$m.model3.json"); do
        [ -f "$dir/$f" ] && continue
        mkdir -p "$(dirname "$dir/$f")"
        curl -sSfL -o "$dir/$f" "$samples/$m/$f" || echo "could not fetch $m/$f"
    done
done

if [ ! -f "$out/core.js" ]; then
    tmp=$(mktemp -d)
    (cd "$tmp" && npm pack l2d@2.1.1 --silent >/dev/null && tar xzf l2d-2.1.1.tgz)
    node "$here/extract-core.mjs" "$tmp/package/dist/index.js" "$out/core.js"
    rm -rf "$tmp"
fi

# The Cubism Framework (for physics), as bundled in the npm package
# "untitled-pixi-live2d-engine".
if [ ! -f "$out/framework.es.js" ]; then
    tmp=$(mktemp -d)
    (cd "$tmp" && npm pack untitled-pixi-live2d-engine@1.4.0 --silent >/dev/null && tar xzf untitled-pixi-live2d-engine-1.4.0.tgz)
    cp "$tmp/package/dist/cubism.es.js" "$out/framework.es.js"
    rm -rf "$tmp"
fi

for m in Haru Hiyori Mark Natori Rice Mao Wanko Ren; do
    node "$here/dump.mjs" "$out/core.js" "$out/models/$m/$m.moc3" "$out/$m" 12
    physics=$(ls "$out/models/$m/"*.physics3.json 2>/dev/null | head -1)
    if [ -n "$physics" ]; then
        node -e "
const d = require('$out/$m.json').parameters;
process.stdout.write(JSON.stringify(d.ids.map((id, i) => ({ id, min: d.min[i], max: d.max[i], default: d.defaults[i] }))));
" > "$out/$m.params.json"
        node "$here/physics.mjs" "$out/core.js" "$out/framework.es.js" "$physics" "$out/$m.params.json" "$out/$m.physics.json"
    fi
done
echo "wrote $out"
