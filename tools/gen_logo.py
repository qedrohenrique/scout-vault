"""Logo do Cofre: um cadeado usando um chapéu de explorador.

Uma única geometria (em coordenadas 256x256) gera:
- logo.svg / logo-NNN.png / cofre.ico — o ícone colorido do app
- mark.svg — o glifo monocromático usado dentro do app (pintado pelo tema)
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(sys.argv[1])
OUT.mkdir(parents=True, exist_ok=True)

TILE_TOP, TILE_BOTTOM = "#14916f", "#0b7d62"
CREAM = "#f7f2e7"
HAT = "#dba54b"
HAT_SHADE = "#c08a32"
BAND = "#5b3d10"
KEY = "#0b7d62"

# ── geometria ────────────────────────────────────────────────────────────────
# Caminhos como listas de comandos: ("M", x, y), ("L", x, y), ("Q", cx, cy, x, y), ("C", ...), ("Z",)
CROWN = [("M", 86, 110), ("C", 87, 86, 92, 66, 104, 60), ("Q", 116, 54, 128, 65),
         ("Q", 140, 54, 152, 60), ("C", 164, 66, 169, 86, 170, 110), ("Z",)]
BAND_SHAPE = [("M", 88.2, 91), ("L", 167.8, 91), ("L", 169.4, 106), ("L", 86.6, 106), ("Z",)]
BRIM = [("M", 34, 114), ("Q", 128, 92, 222, 114), ("Q", 228, 122, 216, 126),
        ("Q", 128, 108, 40, 126), ("Q", 28, 122, 34, 114), ("Z",)]
BODY = (66, 124, 190, 212, 22)  # x0, y0, x1, y1, raio
KEYHOLE_CIRCLE = (128, 158, 12)
KEYHOLE_STEM = (122, 160, 134, 186, 4)


def rrect_cmds(x0, y0, x1, y1, r):
    return [("M", x0 + r, y0), ("L", x1 - r, y0), ("Q", x1, y0, x1, y0 + r), ("L", x1, y1 - r),
            ("Q", x1, y1, x1 - r, y1), ("L", x0 + r, y1), ("Q", x0, y1, x0, y1 - r),
            ("L", x0, y0 + r), ("Q", x0, y0, x0 + r, y0), ("Z",)]


def circle_cmds(cx, cy, r):
    k = 0.5523 * r
    return [("M", cx + r, cy), ("C", cx + r, cy + k, cx + k, cy + r, cx, cy + r),
            ("C", cx - k, cy + r, cx - r, cy + k, cx - r, cy),
            ("C", cx - r, cy - k, cx - k, cy - r, cx, cy - r),
            ("C", cx + k, cy - r, cx + r, cy - k, cx + r, cy), ("Z",)]


def to_d(cmds):
    out = []
    for c in cmds:
        out.append(c[0] + " " + " ".join(f"{v:g}" for v in c[1:]))
    return " ".join(out)


def to_polys(cmds, steps=24):
    polys, cur, pos = [], [], (0, 0)
    for c in cmds:
        op = c[0]
        if op == "M":
            if cur:
                polys.append(cur)
            pos = (c[1], c[2])
            cur = [pos]
        elif op == "L":
            pos = (c[1], c[2])
            cur.append(pos)
        elif op == "Q":
            (x0, y0), (cx, cy), (x, y) = pos, c[1:3], c[3:5]
            for i in range(1, steps + 1):
                t = i / steps
                cur.append(((1 - t) ** 2 * x0 + 2 * (1 - t) * t * cx + t * t * x,
                            (1 - t) ** 2 * y0 + 2 * (1 - t) * t * cy + t * t * y))
            pos = (x, y)
        elif op == "C":
            (x0, y0), (c1x, c1y), (c2x, c2y), (x, y) = pos, c[1:3], c[3:5], c[5:7]
            for i in range(1, steps + 1):
                t = i / steps
                mt = 1 - t
                cur.append((mt ** 3 * x0 + 3 * mt * mt * t * c1x + 3 * mt * t * t * c2x + t ** 3 * x,
                            mt ** 3 * y0 + 3 * mt * mt * t * c1y + 3 * mt * t * t * c2y + t ** 3 * y))
            pos = (x, y)
        elif op == "Z":
            if cur:
                polys.append(cur)
            cur = []
    if cur:
        polys.append(cur)
    return polys


body = rrect_cmds(*BODY)
keyhole = circle_cmds(*KEYHOLE_CIRCLE) + rrect_cmds(*KEYHOLE_STEM)
tile = rrect_cmds(8, 8, 248, 248, 56)

# ── SVG colorido ─────────────────────────────────────────────────────────────
logo_svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{TILE_TOP}"/>
      <stop offset="1" stop-color="{TILE_BOTTOM}"/>
    </linearGradient>
  </defs>
  <path d="{to_d(tile)}" fill="url(#tile)"/>
  <path d="{to_d(body)}" fill="{CREAM}"/>
  <path d="{to_d(keyhole)}" fill="{KEY}"/>
  <path d="{to_d(CROWN)}" fill="{HAT}"/>
  <path d="{to_d(BAND_SHAPE)}" fill="{BAND}"/>
  <path d="{to_d(BRIM)}" fill="{HAT_SHADE}"/>
</svg>
"""
(OUT / "logo.svg").write_text(logo_svg, encoding="utf-8")

# ── glifo monocromático (máscara: o app pinta com a cor do tema) ──────────────
# Fendas transparentes separam aba, faixa e corpo; o buraco da fechadura é vazado.
mark_svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="24 44 208 176" fill="currentColor">
  <path fill-rule="evenodd" d="{to_d(rrect_cmds(66, 132, 190, 212, 20))} {to_d(keyhole)}"/>
  <path d="{to_d([("M", 86, 104), ("C", 87, 86, 92, 66, 104, 60), ("Q", 116, 54, 128, 65), ("Q", 140, 54, 152, 60), ("C", 164, 66, 169, 86, 170, 104), ("Z",)])}"/>
  <path d="{to_d(BRIM)}"/>
</svg>
"""
(OUT / "mark.svg").write_text(mark_svg, encoding="utf-8")

# ── PNG (renderiza grande e reduz) ───────────────────────────────────────────
S = 1024
k = S / 256


def fill(draw, cmds, color):
    for poly in to_polys(cmds):
        draw.polygon([(x * k, y * k) for x, y in poly], fill=color)


big = Image.new("RGBA", (S, S), (0, 0, 0, 0))
# degradê do fundo, recortado pelo quadrado arredondado
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
t0 = tuple(int(TILE_TOP[i:i + 2], 16) for i in (1, 3, 5))
t1 = tuple(int(TILE_BOTTOM[i:i + 2], 16) for i in (1, 3, 5))
for y in range(S):
    t = y / (S - 1)
    gd.line([(0, y), (S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(t0, t1)) + (255,))
mask = Image.new("L", (S, S), 0)
fill(ImageDraw.Draw(mask), tile, 255)
big.paste(grad, (0, 0), mask)
d = ImageDraw.Draw(big)
fill(d, body, CREAM)
fill(d, keyhole, KEY)
fill(d, CROWN, HAT)
fill(d, BAND_SHAPE, BAND)
fill(d, BRIM, HAT_SHADE)

sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256, 512]
imgs = {}
for s in sizes:
    im = big.resize((s, s), Image.LANCZOS)
    imgs[s] = im
    if s in (32, 64, 128, 256, 512):
        im.save(OUT / f"logo-{s}.png")
ico_sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
imgs[256].save(OUT / "cofre.ico", format="ICO", sizes=[(s, s) for s in ico_sizes],
               append_images=[imgs[s] for s in ico_sizes if s != 256])
print("ok", OUT)
