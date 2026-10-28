#!/usr/bin/env python3
"""Draw the Inno Setup wizard graphics from the app icon.

    python scripts/make-installer-art.py

Writes 24-bit BMPs into installer/art/ at several sizes, because Inno Setup picks the one
matching the display scaling. Colours are the app's own dark theme (ui/app.slint):
background #0E0F12 to #16171C, brand accent #6C8CFF -- the DEFAULT accent, not whatever
accent the person building it happens to have chosen in their own settings.

Needs Pillow:  python -m pip install Pillow
"""

import os
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICON = os.path.join(ROOT, 'assets', 'skj-files.png')
OUT = os.path.join(ROOT, 'installer', 'art')

BG_TOP = (14, 15, 18)       # #0E0F12
BG_BOTTOM = (22, 23, 28)    # #16171C
ACCENT = (108, 140, 255)    # #6C8CFF
TEXT = (237, 239, 244)
TEXT_DIM = (150, 156, 170)

# Inno Setup wizard image sizes: 100%, 125%, 150%, 200%
BIG = [(164, 314), (192, 386), (246, 459), (328, 604)]
SMALL = [(55, 58), (64, 68), (83, 80), (110, 116)]


def gradient(size):
    """Vertical dark gradient, the app's own background."""
    w, h = size
    img = Image.new('RGB', size, BG_TOP)
    d = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(1, h - 1)
        d.line([(0, y), (w, y)], fill=tuple(
            round(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3)))
    return img


def glow(img, cx, cy, radius, colour, strength=0.5):
    """A soft accent halo, so the logo does not float on flat black."""
    layer = Image.new('RGB', img.size, (0, 0, 0))
    d = ImageDraw.Draw(layer)
    d.ellipse([cx - radius, cy - radius, cx + radius, cy + radius], fill=colour)
    layer = layer.filter(ImageFilter.GaussianBlur(radius * 0.55))
    return Image.blend(img, Image.blend(img, layer, 1.0), strength * 0.45)


def font(px):
    for name in ('segoeuisemibold.ttf', 'segoeuib.ttf', 'arialbd.ttf'):
        try:
            return ImageFont.truetype(name, px)
        except OSError:
            continue
    return ImageFont.load_default()


def big_image(size):
    w, h = size
    img = gradient(size)
    s = w / 164.0  # everything scales off the 100% width

    logo_px = int(78 * s)
    cx, cy = w // 2, int(h * 0.36)
    img = glow(img, cx, cy, int(logo_px * 0.95), ACCENT, strength=0.55)

    logo = Image.open(ICON).convert('RGBA').resize((logo_px, logo_px), Image.LANCZOS)
    img.paste(logo, (cx - logo_px // 2, cy - logo_px // 2), logo)

    d = ImageDraw.Draw(img)
    name_f, sub_f = font(int(19 * s)), font(int(11 * s))
    name, sub = 'SKJ Files', 'SKJ Tech'

    ny = cy + int(logo_px * 0.78)
    nw = d.textbbox((0, 0), name, font=name_f)[2]
    d.text((cx - nw // 2, ny), name, font=name_f, fill=TEXT)

    # thin accent rule under the wordmark
    ry = ny + int(28 * s)
    d.line([(cx - int(22 * s), ry), (cx + int(22 * s), ry)], fill=ACCENT, width=max(1, int(2 * s)))

    sw = d.textbbox((0, 0), sub, font=sub_f)[2]
    d.text((cx - sw // 2, ry + int(10 * s)), sub, font=sub_f, fill=TEXT_DIM)
    return img


def small_image(size):
    w, h = size
    img = gradient(size)
    px = int(min(w, h) * 0.74)
    logo = Image.open(ICON).convert('RGBA').resize((px, px), Image.LANCZOS)
    img.paste(logo, ((w - px) // 2, (h - px) // 2), logo)
    return img


def main():
    if not os.path.exists(ICON):
        raise SystemExit('missing ' + ICON)
    os.makedirs(OUT, exist_ok=True)
    made = []
    for i, size in enumerate(BIG):
        p = os.path.join(OUT, 'wizard%s.bmp' % ('' if i == 0 else '-%d' % (i + 1)))
        big_image(size).save(p, 'BMP')
        made.append((p, size))
    for i, size in enumerate(SMALL):
        p = os.path.join(OUT, 'wizard-small%s.bmp' % ('' if i == 0 else '-%d' % (i + 1)))
        small_image(size).save(p, 'BMP')
        made.append((p, size))
    for p, size in made:
        print('%-44s %dx%d  %d bytes' % (os.path.relpath(p, ROOT), size[0], size[1], os.path.getsize(p)))


if __name__ == '__main__':
    main()
