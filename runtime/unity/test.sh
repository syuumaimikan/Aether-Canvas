#!/bin/sh
# Test the Unity package without Unity:
#   1. the C# binding runs against the native library with .NET 8 and must
#      reproduce the native player exactly (geometry at every reference
#      pose, draw list, motions, face tracking, lip sync);
#   2. every script (runtime and editor) compiles against Unity's own
#      UnityEngine/UnityEditor assemblies with Unity's C# version;
#   3. the HLSL in every shader compiles (glslang), for both colour spaces.
# Needs dotnet (8.0) and glslangValidator. Drawing inside Unity is not
# tested here: that needs a Unity licence.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
package="$here/com.aethercanvas.player"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1

(cd "$root" && cargo build -q --release -p aether-player)
if [ ! -f "$root/runtime/web/model/model.json" ]; then
    "$root/runtime/web/build.sh" --demo
fi

echo "# the C# binding, against the native library"
dotnet build -c Release -v quiet -o "$out/binding" "$package/Tests~/Binding/Binding.csproj"
cp "$root"/target/release/libaether_player.* "$out/binding/" 2>/dev/null || true
cp "$root"/target/release/aether_player.dll "$out/binding/" 2>/dev/null || true
dotnet "$out/binding/Binding.dll" "$root/runtime/web/model" "$root/runtime/web/test/reference"

echo "# the package's scripts, against Unity's assemblies"
dotnet build -v quiet -o "$out/unity" "$package/Tests~/UnityCompile/UnityCompile.csproj"

echo "# the shaders' HLSL"
python3 "$package/Tests~/check_shaders.py"
