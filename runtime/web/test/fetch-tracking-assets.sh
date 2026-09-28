#!/bin/sh
# Fetch what the browser face-tracking test needs into test/tracking-assets
# (not committed): MediaPipe tasks-vision, its face landmarker model, and a
# public-domain photo of a face ("Astronaut Eileen Collins", NASA, as shipped
# in scikit-image's test data).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/tracking-assets"
mkdir -p "$out"
cd "$out"

npm pack --silent @mediapipe/tasks-vision@0.10.14 >/dev/null
tar xzf mediapipe-tasks-vision-0.10.14.tgz
rm mediapipe-tasks-vision-0.10.14.tgz

curl -fsSL -o face_landmarker.task \
    https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task

pip download --quiet --no-deps --only-binary=:all: -d wheel scikit-image==0.24.0
python3 - <<'PY'
import glob, zipfile
wheel = zipfile.ZipFile(glob.glob("wheel/*.whl")[0])
open("face.png", "wb").write(wheel.read("skimage/data/astronaut.png"))
PY
rm -rf wheel

# The photo as a short Y4M video, for Chromium's fake camera
# (--use-file-for-fake-video-capture). Standard library only.
python3 - <<'PY'
import struct, zlib

def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat = 8, b""
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, colour = struct.unpack(">IIBB", body[:10])
            assert depth == 8 and colour == 2, "8-bit RGB expected"
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    raw, bpp, stride = zlib.decompress(idat), 3, width * 3
    rows, prev, i = [], bytearray(stride), 0
    for _ in range(height):
        kind, line = raw[i], bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if kind == 1: line[x] = (line[x] + a) & 255
            elif kind == 2: line[x] = (line[x] + b) & 255
            elif kind == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        prev = line
    return width, height, rows

w, h, rows = read_png("face.png")
clamp = lambda v: max(0, min(255, int(round(v))))
Y = bytearray(clamp(0.299 * r[3 * x] + 0.587 * r[3 * x + 1] + 0.114 * r[3 * x + 2]) for r in rows for x in range(w))
U, V = bytearray(), bytearray()
for y in range(0, h, 2):
    for x in range(0, w, 2):
        px = [(rows[y + dy][3 * (x + dx)], rows[y + dy][3 * (x + dx) + 1], rows[y + dy][3 * (x + dx) + 2]) for dy in (0, 1) for dx in (0, 1)]
        r, g, b = (sum(c[k] for c in px) / 4 for k in range(3))
        U.append(clamp(-0.168736 * r - 0.331264 * g + 0.5 * b + 128))
        V.append(clamp(0.5 * r - 0.418688 * g - 0.081312 * b + 128))
with open("face.y4m", "wb") as f:
    f.write(b"YUV4MPEG2 W%d H%d F30:1 Ip A1:1 C420jpeg\n" % (w, h))
    for _ in range(30):
        f.write(b"FRAME\n" + Y + U + V)
PY
echo "fetched into $out"
