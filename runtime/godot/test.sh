#!/bin/sh
# Run the Godot tests. Needs Godot 4.3 or later on PATH (or in $GODOT).
#
# Logic runs headless. When xvfb-run is available, the scene is also drawn
# with Godot's OpenGL renderer (Mesa's software rasteriser is enough) and
# compared, pixel by pixel, with the software player's reference renders.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
godot=${GODOT:-godot}

"$here/build.sh"
# Let Godot import the project (textures, the extension) once.
"$godot" --headless --path "$here" --import >/dev/null 2>&1 || true

"$godot" --headless --audio-driver Dummy --path "$here" -s res://test/run_tests.gd

if command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a -s "-screen 0 1024x768x24" \
        "$godot" --audio-driver Dummy --path "$here" --rendering-driver opengl3 -s res://test/run_tests.gd -- --render
else
    echo "xvfb-run not found; skipping the rendering comparison"
fi
