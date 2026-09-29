#!/usr/bin/env python3
"""Prepare Luna's layers from her illustration and her parts sheet.

    python3 crates/aether-samples/tools/prepare_luna.py

Inputs (in crates/aether-samples/assets/luna/):
  character.png    the finished illustration, background removed
  parts-sheet.png  the same character drawn as separate parts

Outputs (in assets/luna/layers/, every layer the illustration's size):
  face_cover.png           skin over the illustration's own eyes and mouth
  eye_white_{L,R}.png      the sheet's eyes, fitted onto the face, without iris
  iris_{L,R}.png           their irises, to move and to clip to the whites
  lash_{L,R}.png           their lashes, above the irises
  mouth.png                the illustration's own mouth, to fade out
  mouth_*.png              the sheet's mouth shapes on skin, colour-matched
                           to the face
  cheek.png                a blush to fade in
  hair_over_eyes.png       the illustration's strands of hair over the eyes,
                           to keep them in front
  layout.json              where things are, for the rig

L and R are the character's: her left eye is on the viewer's right.
Needs Pillow and NumPy. The result is deterministic.
"""

import json
import os
from collections import deque

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "..", "assets", "luna")
OUT = os.path.join(ASSETS, "layers")



def normalise_base(path):
    """Tidy the illustration once, in place: the background remover left the
    figure at 98% opacity and colour under fully transparent pixels. Lift
    near-opaque alpha to opaque and clear hidden colour; running it again
    changes nothing."""
    a = np.asarray(Image.open(path).convert("RGBA")).copy()
    alpha = a[..., 3]
    tidy = a.copy()
    tidy[alpha >= 248, 3] = 255
    tidy[alpha == 0] = 0
    if not np.array_equal(tidy, a):
        Image.fromarray(tidy, "RGBA").save(path, optimize=True)
        print("normalised", path)


normalise_base(os.path.join(ASSETS, "character.png"))
base_img = Image.open(os.path.join(ASSETS, "character.png")).convert("RGBA")
sheet = Image.open(os.path.join(ASSETS, "parts-sheet.png")).convert("RGBA")
W, H = base_img.size
BASE = np.asarray(base_img).astype(np.float32) / 255.0

# Parts on the sheet (pixel boxes) and where they go on the face.
EYES = {
    # Viewer's left is her right eye.
    "R": {"box": (540, 241, 597, 278), "guess": (479, 200)},
    "L": {"box": (620, 242, 673, 278), "guess": (567, 189)},
}
# Fitted to the face first, to find the mouth shapes' scale.
MOUTH_FIT = (554, 320, 592, 338)
MOUTHS = {
    "line": (555, 346, 593, 365),
    "open": (604, 346, 641, 365),
    "wide": (605, 372, 639, 390),
    "o": (664, 294, 682, 309),
}
MOUTH_CENTRE = (528.0, 238.0)


def sprite(box):
    return sheet.crop(box)


def transformed(spr, scale, angle, flip=False):
    if flip:
        spr = spr.transpose(Image.FLIP_LEFT_RIGHT)
    w, h = spr.size
    big = spr.resize((max(1, round(w * scale)), max(1, round(h * scale))), Image.LANCZOS)
    return big.rotate(angle, resample=Image.BICUBIC, expand=True)


def on_canvas(spr, cx, cy):
    """An RGBA float canvas with `spr` centred at (cx, cy)."""
    layer = Image.new("RGBA", (W, H))
    layer.alpha_composite(spr, (int(round(cx - spr.width / 2)), int(round(cy - spr.height / 2))))
    return np.asarray(layer).astype(np.float32) / 255.0


def ncc(spr, cx, cy):
    a = np.asarray(spr).astype(np.float32) / 255.0
    h, w = a.shape[:2]
    x0, y0 = int(round(cx - w / 2)), int(round(cy - h / 2))
    reg = BASE[y0:y0 + h, x0:x0 + w, :3]
    if reg.shape[:2] != (h, w):
        return -1.0
    m = a[..., 3:4]
    total = m.sum() + 1e-6
    ds = (a[..., :3] - (a[..., :3] * m).sum((0, 1)) / total) * m
    dr = (reg - (reg * m).sum((0, 1)) / total) * m
    return float((ds * dr).sum() / np.sqrt((ds * ds).sum() * (dr * dr).sum() + 1e-9))


def fit(box, guess, scales, angles, radius):
    """Scale, angle and centre where a sheet part best matches the face."""
    spr = sprite(box)
    best = (-2.0, None)
    for s in scales:
        for a in angles:
            t = transformed(spr, s, a)
            for dy in range(-radius, radius + 1, 2):
                for dx in range(-radius, radius + 1, 2):
                    score = ncc(t, guess[0] + dx, guess[1] + dy)
                    if score > best[0]:
                        best = (score, (s, a, guess[0] + dx, guess[1] + dy))
    score, (s, a, cx, cy) = best
    for s2 in np.linspace(s - 0.03, s + 0.03, 4):
        for a2 in np.linspace(a - 1.5, a + 1.5, 4):
            t = transformed(spr, s2, a2)
            for dy in range(-2, 3):
                for dx in range(-2, 3):
                    sc = ncc(t, cx + dx, cy + dy)
                    if sc > best[0]:
                        best = (sc, (float(s2), float(a2), cx + dx, cy + dy))
    return best


def save(name, rgba):
    px = (np.clip(rgba, 0, 1) * 255 + 0.5).astype(np.uint8)
    # Colour under transparent pixels is invisible; clearing it keeps the
    # files small.
    px[px[..., 3] == 0] = 0
    img = Image.fromarray(px, "RGBA")
    img.save(os.path.join(OUT, name + ".png"), optimize=True)


def dilate(mask, r):
    out = mask.copy()
    for _ in range(r):
        grown = out.copy()
        grown[1:, :] |= out[:-1, :]
        grown[:-1, :] |= out[1:, :]
        grown[:, 1:] |= out[:, :-1]
        grown[:, :-1] |= out[:, 1:]
        out = grown
    return out


def blur(a, passes=1):
    for _ in range(passes):
        p = np.pad(a, 1, mode="edge")
        a = (p[:-2, 1:-1] + p[2:, 1:-1] + p[1:-1, :-2] + p[1:-1, 2:] + 2 * p[1:-1, 1:-1]) / 6.0
    return a


SKIN = np.median(BASE[228:236, 498:512, :3].reshape(-1, 3), axis=0)


def inpaint_skin(region):
    """Fill `region` with skin diffused in from the skin around it, ignoring
    hair and lines on its border."""
    rgb = BASE[..., :3].copy()
    ys, xs = np.nonzero(region)
    y0, y1, x0, x1 = ys.min() - 2, ys.max() + 3, xs.min() - 2, xs.max() + 3
    sub = rgb[y0:y1, x0:x1].copy()
    reg = region[y0:y1, x0:x1]
    skin_like = (np.abs(sub - SKIN).max(axis=2) < 0.12) & ~reg
    known = skin_like.astype(np.float32)
    value = np.where(reg[..., None], SKIN, sub)
    for _ in range(600):
        p = np.pad(value, ((1, 1), (1, 1), (0, 0)), mode="edge")
        k = np.pad(known + reg, 1, mode="constant")[..., None]
        num = (p[:-2, 1:-1] * k[:-2, 1:-1] + p[2:, 1:-1] * k[2:, 1:-1]
               + p[1:-1, :-2] * k[1:-1, :-2] + p[1:-1, 2:] * k[1:-1, 2:])
        den = k[:-2, 1:-1] + k[2:, 1:-1] + k[1:-1, :-2] + k[1:-1, 2:]
        avg = num / np.maximum(den, 1e-6)
        value = np.where(reg[..., None], avg, value)
    out = np.zeros((H, W, 4), np.float32)
    out[y0:y1, x0:x1, :3] = value
    out[..., 3] = region
    return out


def largest_blob(mask):
    h, w = mask.shape
    seen = np.zeros_like(mask)
    best = []
    for y, x in zip(*np.nonzero(mask)):
        if seen[y, x]:
            continue
        q = deque([(y, x)])
        seen[y, x] = True
        pts = []
        while q:
            cy, cx = q.popleft()
            pts.append((cy, cx))
            for ny, nx in ((cy + 1, cx), (cy - 1, cx), (cy, cx + 1), (cy, cx - 1)):
                if 0 <= ny < h and 0 <= nx < w and mask[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    q.append((ny, nx))
        if len(pts) > len(best):
            best = pts
    return best


def main():
    os.makedirs(OUT, exist_ok=True)
    layout = {"size": [W, H], "eyes": {}, "mouth": {}}
    cover_region = np.zeros((H, W), bool)
    openings = np.zeros((H, W), bool)
    yy, xx = np.mgrid[0:H, 0:W]

    for side, spec in EYES.items():
        score, (s, a, cx, cy) = fit(spec["box"], spec["guess"], np.linspace(0.8, 1.0, 5), [0, 3, 6, 9], 8)
        print(f"eye {side}: match {score:.3f} at scale {s:.3f}, {a:.1f} deg, ({cx}, {cy})")
        eye = on_canvas(transformed(sprite(spec["box"]), s, a), cx, cy)
        alpha = eye[..., 3]
        cover_region |= dilate(alpha > 0.05, 4)
        r, g, b = eye[..., 0], eye[..., 1], eye[..., 2]
        lum = 0.299 * r + 0.587 * g + 0.114 * b
        blue = (b - r > 0.08) & (b > 0.3) & (alpha > 0.5)
        pts = largest_blob(blue)
        py = np.array([p[0] for p in pts])
        px = np.array([p[1] for p in pts])
        icx, icy = (px.min() + px.max()) / 2.0, (py.min() + py.max()) / 2.0
        irx, iry = (px.max() - px.min()) / 2.0 + 1.0, (py.max() - py.min()) / 2.0 + 1.0
        # The top of the iris is dark under the lashes and not blue enough
        # to be found: an iris is a little taller than wide.
        tall = max(iry, irx * 1.15)
        icy -= tall - iry
        iry = tall
        d = np.sqrt(((xx - icx) / irx) ** 2 + ((yy - icy) / iry) ** 2)
        inside = np.clip((1.0 - d) * min(irx, iry) + 0.5, 0.0, 1.0)
        top_band = (yy < icy - 0.35 * iry).astype(np.float32)
        lash = np.clip((0.55 - lum) / 0.25, 0.0, 1.0) * (r >= b - 0.02)
        lash = lash * np.maximum(1.0 - inside, top_band) * alpha
        light = (lum > 0.8) & (alpha > 0.9) & (inside < 0.01)
        sclera = np.median(eye[light][:, :3], axis=0) if light.any() else np.array([0.95, 0.95, 1.0])

        # The upper lid: the bottom edge of the upper lashes, per column.
        # A closing eye slides the lashes down to meet the lower lid.
        xs_a = np.nonzero((alpha > 0.05).any(axis=0))[0]
        upper = []
        for x in range(xs_a.min(), xs_a.max() + 1):
            col = np.nonzero(lash[:, x] > 0.45)[0]
            if len(col) == 0:
                continue
            y = col[0]
            while y + 1 < H and lash[y + 1, x] > 0.45:
                y += 1
            upper.append((float(x), y + 1.0))
        ux = np.array([u[0] for u in upper])
        uy = np.array([u[1] for u in upper])
        uy = np.array([np.median(uy[max(0, i - 2):i + 3]) for i in range(len(uy))])
        uy = np.array([uy[max(0, i - 3):i + 4].mean() for i in range(len(uy))])
        lid_y = np.interp(np.arange(W), ux, uy)[None, :]

        # Split along the lid: above it everything is lashes (and the lid),
        # below it the iris and, under the iris, sclera shaded by the lid
        # toward the top, so moving the iris reveals an eye, not a hole.
        below = np.clip(yy - lid_y + 0.5, 0.0, 1.0)
        shade = np.clip((yy - lid_y + 1.0) / (0.9 * iry), 0.0, 1.0)
        shade = (shade * shade * (3.0 - 2.0 * shade))[..., None]
        fill = sclera * np.array([0.62, 0.65, 0.8]) * (1.0 - shade) + sclera * shade
        white = eye.copy()
        mix = (inside * below)[..., None]
        white[..., :3] = eye[..., :3] * (1 - mix) + fill * mix
        iris = eye.copy()
        iris[..., 3] = alpha * inside * below
        lash_layer = eye.copy()
        lash_layer[..., 3] = np.maximum(alpha * (1.0 - below), lash * (1.0 - inside))
        save(f"eye_white_{side}", white)
        save(f"iris_{side}", iris)
        save(f"lash_{side}", lash_layer)

        ys, xs = np.nonzero(alpha > 0.05)
        bounds = [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1]
        # The lower lid: where the white of the eye ends.
        # Sclera is light and cool; the skin around it is warm.
        whites = (lum > 0.72) & (b >= r - 0.03) & (alpha > 0.5) & (lash < 0.3)
        # The opening between the lids, where hair cannot be told from
        # the whites.
        openings |= dilate((whites | (inside > 0.5)) & (alpha > 0.3), 2)
        wy = np.nonzero(whites)[0]
        lid = float(np.percentile(wy, 97)) if len(wy) else bounds[3] - 4.0
        # The corners of the opening, where a closed eye's line ends: the
        # outer one is away from the middle of the face.
        wys, wxs = np.nonzero(whites)
        ends = []
        for x_end in (wxs.min(), wxs.max()):
            near = np.abs(wxs - x_end) <= 2
            ends.append([float(x_end), float(wys[near].mean())])
        outer, inner = (ends[0], ends[1]) if side == "R" else (ends[1], ends[0])
        dark = lash > 0.8
        lash_rgb = np.median(eye[dark][:, :3], axis=0) if dark.any() else np.array([0.25, 0.15, 0.15])
        layout["eyes"][side] = {
            "lash_colour": [float(v) for v in lash_rgb],
            "bounds": bounds,
            "iris": [float(icx), float(icy), float(irx), float(iry)],
            "lid": lid,
            "outer": outer,
            "inner": inner,
            "upper": [[x, float(y)] for x, y in zip(ux[::2], uy[::2])],
            "angle": float(a),
        }


    # Strands of the bangs that fall over the eyes: the illustration's own
    # hair where the eyes and their skin cover it, outside the openings. It
    # goes above the eyes, so a blinking eye stays behind the hair.
    br, bg_, bb = BASE[..., 0], BASE[..., 1], BASE[..., 2]
    blum = 0.299 * br + 0.587 * bg_ + 0.114 * bb
    sat = BASE[..., :3].max(axis=2) - BASE[..., :3].min(axis=2)
    hairy = (sat < 0.12) & (blum > 0.68) & (br - bb < 0.035) & (BASE[..., 3] > 0.5)
    strands = hairy & dilate(cover_region, 1) & ~openings
    strands = blur(strands.astype(np.float32), 1) > 0.5
    over = BASE.copy()
    over[..., 3] = np.clip(blur(strands.astype(np.float32), 1) * 1.4, 0, 1) * BASE[..., 3]
    save("hair_over_eyes", over)

    # Mouths: fit the closed smile, then put every shape at its place.
    score, (ms, ma, mx, my) = fit(MOUTH_FIT, MOUTH_CENTRE, np.linspace(0.6, 1.1, 6), [0, 4, 8], 6)
    print(f"mouth: match {score:.3f} at scale {ms:.3f}, {ma:.1f} deg, ({mx}, {my})")
    layout["mouth"] = {"centre": [float(mx), float(my)], "scale": float(ms), "angle": float(ma)}
    yy_m = ((yy - my) / 9.0) ** 2 + ((xx - mx) / 17.0) ** 2
    cover_region |= yy_m <= 1.0
    for name, box in MOUTHS.items():
        scale = ms * (0.8 if name == "o" else 1.0)
        m = on_canvas(transformed(sprite(box), scale, ma), mx, my + 2)
        # Match the patch's skin to the face's: compare where the patch
        # is mostly opaque but has no mouth drawn (its outer ring).
        a = m[..., 3]
        ring = (a > 0.4) & (a < 0.95)
        if ring.sum() > 20:
            gain = BASE[ring][:, :3].mean(axis=0) / np.maximum(m[ring][:, :3].mean(axis=0), 1e-3)
            m[..., :3] = np.clip(m[..., :3] * gain, 0, 1)
        save(f"mouth_{name}", m)

    save("face_cover", inpaint_skin(cover_region))
    # The illustration's own mouth, feathered, for the resting face.
    own = BASE.copy()
    edge = np.clip((1.0 - np.sqrt(yy_m)) * 6.0, 0.0, 1.0)
    own[..., 3] = BASE[..., 3] * edge
    save("mouth", own)

    # A blush under each eye, tilted with the face.
    cheek = np.zeros((H, W, 4), np.float32)
    for side in ("L", "R"):
        e = layout["eyes"][side]
        cx, cy = (e["bounds"][0] + e["bounds"][2]) / 2.0, e["lid"] + 14.0
        dd = ((xx - cx) / 20.0) ** 2 + ((yy - cy) / 8.0) ** 2
        a = np.exp(-dd * 1.6) * 0.55
        cheek[..., 3] = np.maximum(cheek[..., 3], a)
    cheek[..., :3] = np.array([1.0, 0.52, 0.6])
    save("cheek", cheek)

    with open(os.path.join(OUT, "layout.json"), "w") as f:
        json.dump(layout, f, indent=2)
    print("wrote", OUT)


if __name__ == "__main__":
    main()
