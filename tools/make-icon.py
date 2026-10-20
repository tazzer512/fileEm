"""Draws the fileEm app icon and writes every size and format the app and its packages use."""
import os
import shutil
from PIL import Image, ImageDraw, ImageFilter

S = 1024  # master canvas, downscaled per size

def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))

def gradient(size, c1, c2):
    """Diagonal gradient top-left c1 -> bottom-right c2."""
    w, h = size
    g = Image.new("RGBA", size)
    px = g.load()
    for y in range(h):
        for x in range(w):
            t = (x + y) / (w + h - 2)
            px[x, y] = lerp(c1, c2, t)
    return g

def vgradient(size, c1, c2):
    w, h = size
    g = Image.new("RGBA", size)
    d = ImageDraw.Draw(g)
    for y in range(h):
        d.line([(0, y), (w, y)], fill=lerp(c1, c2, y / max(1, h - 1)))
    return g

def masked(fill, mask):
    out = Image.new("RGBA", fill.size, (0, 0, 0, 0))
    out.paste(fill, (0, 0), mask)
    return out

def folder_mask(x, y, w, h, r, tab_w, tab_h):
    m = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(m)
    # tab
    d.rounded_rectangle([x, y, x + tab_w, y + tab_h * 2], r, fill=255)
    d.polygon([(x + tab_w - r, y), (x + tab_w + tab_h * 0.9, y + tab_h), (x + tab_w - r, y + tab_h)], fill=255)
    # body
    d.rounded_rectangle([x, y + tab_h, x + w, y + h], r, fill=255)
    return m

def master():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # tile
    tile_mask = Image.new("L", (S, S), 0)
    m = 40
    ImageDraw.Draw(tile_mask).rounded_rectangle([m, m, S - m, S - m], 220, fill=255)
    img = Image.alpha_composite(img, masked(gradient((S, S), (76, 110, 245, 255), (132, 66, 220, 255)), tile_mask))
    # soft gloss at the top of the tile
    gloss = Image.new("L", (S, S), 0)
    ImageDraw.Draw(gloss).ellipse([-200, -620, S + 200, 430], fill=38)
    gloss = gloss.filter(ImageFilter.GaussianBlur(70))
    gloss = Image.composite(gloss, Image.new("L", (S, S), 0), tile_mask)
    img = Image.alpha_composite(img, masked(Image.new("RGBA", (S, S), (255, 255, 255, 255)), gloss))

    # back folder (the second pane), offset up-right
    back = folder_mask(300, 230, 500, 400, 50, 190, 70)
    img = Image.alpha_composite(img, masked(Image.new("RGBA", (S, S), (255, 255, 255, 110)), back))

    # shadow of the front folder
    front = folder_mask(200, 330, 560, 450, 56, 210, 80)
    sh = front.filter(ImageFilter.GaussianBlur(28)).point(lambda v: v * 0.45)
    sh = sh.transform((S, S), Image.AFFINE, (1, 0, -10, 0, 1, -24))
    img = Image.alpha_composite(img, masked(Image.new("RGBA", (S, S), (30, 10, 80, 255)), sh))

    # front folder: amber gradient
    img = Image.alpha_composite(img, masked(vgradient((S, S), (255, 214, 102, 255), (247, 159, 31, 255)), front))
    # front flap (lighter, slightly lower than the tab line) for depth
    flap = Image.new("L", (S, S), 0)
    ImageDraw.Draw(flap).rounded_rectangle([200, 430, 760, 780], 56, fill=255)
    img = Image.alpha_composite(img, masked(vgradient((S, S), (255, 228, 140, 255), (250, 176, 5, 255)), flap))
    # two "pane" lines on the flap: the dual-pane hint
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([280, 540, 460, 572], 16, fill=(255, 255, 255, 200))
    d.rounded_rectangle([280, 620, 420, 652], 16, fill=(255, 255, 255, 150))
    d.rounded_rectangle([510, 540, 680, 572], 16, fill=(255, 255, 255, 200))
    d.rounded_rectangle([510, 620, 640, 652], 16, fill=(255, 255, 255, 150))
    d.rounded_rectangle([478, 500, 492, 740], 7, fill=(214, 120, 0, 140))
    return img

# Run from the repository root: python3 tools/make-icon.py
big = master()
res = os.path.join("src", "main", "resources", "com", "fileem", "icons")
pkg = os.path.join("src", "packaging")
os.makedirs(res, exist_ok=True)
os.makedirs(pkg, exist_ok=True)
# Window icons, loaded by Icons.appIcons()
for s in (16, 24, 32, 48, 64, 128, 256, 512):
    big.resize((s, s), Image.LANCZOS).save(os.path.join(res, f"fileem-{s}.png"), optimize=True)
# Packaging icons for jpackage: PNG (Linux), ICO (Windows), ICNS (macOS)
big.resize((512, 512), Image.LANCZOS).save(os.path.join(pkg, "fileem.png"), optimize=True)
big.resize((256, 256), Image.LANCZOS).save(os.path.join(pkg, "fileem.ico"),
                                          sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
big.save(os.path.join(pkg, "fileem.icns"))
# The Rust app (crates/fileem): the same sizes for its window icon and packages
rust = os.path.join("crates", "fileem", "icons")
os.makedirs(rust, exist_ok=True)
for s in (16, 24, 32, 48, 64, 128, 256, 512):
    shutil.copy(os.path.join(res, f"fileem-{s}.png"), rust)
for f in ("fileem.ico", "fileem.icns"):
    shutil.copy(os.path.join(pkg, f), rust)
