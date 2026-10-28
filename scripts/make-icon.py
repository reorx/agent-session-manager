# /// script
# requires-python = ">=3.12"
# dependencies = ["resvg-py==0.5.0"]
# ///
"""Render the app icon SVGs into an .iconset and pack it into assets/icon/AppIcon.icns.

Images of 32 px and below come from the simplified small artwork, the rest from the full one.
The .iconset stays under tmp/icon/ for inspection.
"""

import subprocess
from pathlib import Path

import resvg_py

ROOT = Path(__file__).resolve().parent.parent
ICON_DIR = ROOT / 'assets' / 'icon'
FULL = ICON_DIR / 'app-icon.svg'
SMALL = ICON_DIR / 'app-icon-small.svg'
ICNS = ICON_DIR / 'AppIcon.icns'
ICONSET = ROOT / 'tmp' / 'icon' / 'AppIcon.iconset'

# The (points, scale) slots iconutil expects in an .iconset.
SLOTS = [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)]
SMALL_MAX_PX = 32


def slot_name(points, scale):
    suffix = '@2x' if scale == 2 else ''
    return f'icon_{points}x{points}{suffix}.png'


def render(src, px):
    return bytes(resvg_py.svg_to_bytes(svg_path=str(src), width=px, height=px))


def main():
    ICONSET.mkdir(parents=True, exist_ok=True)
    for points, scale in SLOTS:
        px = points * scale
        src = SMALL if px <= SMALL_MAX_PX else FULL
        (ICONSET / slot_name(points, scale)).write_bytes(render(src, px))
    subprocess.run(['iconutil', '-c', 'icns', '-o', str(ICNS), str(ICONSET)], check=True)
    print(f'wrote {ICNS.relative_to(ROOT)} (images in {ICONSET.relative_to(ROOT)})')


if __name__ == '__main__':
    main()
