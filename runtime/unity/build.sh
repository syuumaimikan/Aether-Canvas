#!/bin/sh
# Build the native player library into the Unity package's Plugins folder
# (for this platform; run it on each platform you ship).
#
#   runtime/unity/build.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
package="$here/com.aethercanvas.player"

(cd "$root" && cargo build --release -p aether-player)
mkdir -p "$package/Runtime/Plugins/x86_64"
for lib in libaether_player.so libaether_player.dylib aether_player.dll; do
    if [ -f "$root/target/release/$lib" ]; then
        cp "$root/target/release/$lib" "$package/Runtime/Plugins/x86_64/"
        echo "copied $lib into $package/Runtime/Plugins/x86_64"
    fi
done
