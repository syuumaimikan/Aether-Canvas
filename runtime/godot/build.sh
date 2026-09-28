#!/bin/sh
# Build the Godot extension into addons/aether/bin, and put the demo model
# (and its reference renders, for the tests) into the project.
#
#   runtime/godot/build.sh            debug build
#   runtime/godot/build.sh --release  optimised build
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

profile=debug
flag=
if [ "${1:-}" = "--release" ]; then
    profile=release
    flag=--release
fi

(cd "$here/rust" && cargo build $flag)
mkdir -p "$here/addons/aether/bin"
for lib in libaether_godot.so libaether_godot.dylib aether_godot.dll; do
    if [ -f "$here/rust/target/$profile/$lib" ]; then
        cp "$here/rust/target/$profile/$lib" "$here/addons/aether/bin/"
    fi
done

# The demo character, exported by the web runtime's build.
if [ ! -f "$root/runtime/web/model/model.json" ]; then
    "$root/runtime/web/build.sh" --demo
fi
rm -rf "$here/model" "$here/test/reference" "$here/test/fixture"
cp -r "$root/runtime/web/model" "$here/model"
cp -r "$root/runtime/web/test/reference" "$here/test/reference"
# A scene with every drawing path (clipping, every blend mode, tint,
# opacity), with the software player's renders, for the tests.
(cd "$root" && cargo run -q -p aether-player-wgpu --example draw_fixture -- "$here/test/fixture")
echo "built the extension and copied the demo model into $here"
