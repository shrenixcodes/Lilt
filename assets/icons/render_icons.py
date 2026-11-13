"""Renders Lilt's application and notification-area icons.

Run from the repository root after changing the artwork:

    python assets/icons/render_icons.py

Requires Pillow. Output files are committed, so building Lilt does not need Python.
"""

from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
SUPERSAMPLE = 4

PAPER_TOP = (246, 246, 248, 255)
PAPER_BOTTOM = (226, 226, 230, 255)
ISLAND = (12, 12, 14, 255)
LIVE = (95, 207, 143, 255)


def app_icon(size: int) -> Image.Image:
    """An off-white tile with a black island near the top and a small live dot."""
    s = size * SUPERSAMPLE
    unit = s / 256
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    gradient = Image.new("RGBA", (s, s))
    top, bottom = PAPER_TOP, PAPER_BOTTOM
    for y in range(s):
        t = y / (s - 1)
        row = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(4))
        ImageDraw.Draw(gradient).line([(0, y), (s, y)], fill=row)
    mask = Image.new("L", (s, s), 0)
    inset = 10 * unit
    ImageDraw.Draw(mask).rounded_rectangle([inset, inset, s - inset, s - inset], radius=58 * unit, fill=255)
    img.paste(gradient, (0, 0), mask)

    draw = ImageDraw.Draw(img)
    width, height = 156 * unit, 54 * unit
    left, top_edge = (s - width) / 2, 58 * unit
    draw.rounded_rectangle([left, top_edge, left + width, top_edge + height], radius=height / 2, fill=ISLAND)
    r = 9.5 * unit
    cx, cy = left + width - height / 2 - 4 * unit, top_edge + height / 2
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=LIVE)

    return img.resize((size, size), Image.LANCZOS)


def tray_icon(size: int, color: tuple) -> Image.Image:
    """A solid pill glyph with a cut-out dot, legible at 16 px."""
    s = size * SUPERSAMPLE
    unit = s / 32
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    width, height = 28 * unit, 13 * unit
    left, top = (s - width) / 2, (s - height) / 2
    draw.rounded_rectangle([left, top, left + width, top + height], radius=height / 2, fill=color)
    r = 2.6 * unit
    cx, cy = left + width - height / 2, top + height / 2
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(0, 0, 0, 0))
    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    images = [app_icon(n) for n in sizes]
    images[-1].save(HERE / "lilt.png")
    images[-1].save(HERE / "lilt.ico", sizes=[(n, n) for n in sizes], append_images=images[:-1])
    tray_icon(32, (245, 245, 247, 255)).save(HERE / "tray-light.png")
    tray_icon(32, (20, 20, 22, 255)).save(HERE / "tray-dark.png")


if __name__ == "__main__":
    main()
