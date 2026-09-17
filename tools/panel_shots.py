"""Render the README panel screenshots.

Runs the SHIPPED exe once per language in its --panel-snapshot mode, which
draws the accounts panel filled with INVENTED accounts (never a real
character), and converts each bottom-up BMP the exe writes into the PNG the
README serves as docs/panel-<lang>.png.

    python tools/panel_shots.py

Scale is forced so a 96-DPI CI runner produces the same picture as a
120-DPI desk. The BMP the exe writes is 32bpp bottom-up; this reads it and
writes a straight PNG (no third-party image library, matching the app,
which carries no image decoder of its own).
"""

import argparse
import os
import struct
import subprocess
import sys
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "release", "DoSwitch.exe")
DOCS = os.path.join(ROOT, "docs")
CASES = ["en", "fr", "es"]
SCALE = "1.25"
# Matches the panel's own drawn corner radius at this scale (14 logical px).
CORNER_RADIUS = 18


def round_corners(width, height, rgba, radius):
    """Make the four corners transparent, so the shipped panel is a clean
    floating card. PrintWindow copies the window as a flat rectangle - it
    does not capture the OS corner rounding - so the raw capture has a hard
    square bottom that reads as a block border against the page. Cutting a
    rounded-corner alpha mask (with 1px anti-aliasing) restores the rounded
    look the app actually has on screen."""
    px = bytearray(rgba)
    r = radius
    for dy in range(r):
        for dx in range(r):
            dist = ((r - dx - 0.5) ** 2 + (r - dy - 0.5) ** 2) ** 0.5
            if dist <= r:
                continue
            coverage = max(0.0, min(1.0, r + 1 - dist))
            for x, y in (
                (dx, dy),
                (width - 1 - dx, dy),
                (dx, height - 1 - dy),
                (width - 1 - dx, height - 1 - dy),
            ):
                i = (y * width + x) * 4
                px[i + 3] = int(px[i + 3] * coverage)
    return bytes(px)


def read_bmp(path):
    """32bpp bottom-up BMP in, (width, height, top-down RGBA) out."""
    data = open(path, "rb").read()
    offset = struct.unpack_from("<I", data, 10)[0]
    width = struct.unpack_from("<i", data, 18)[0]
    height = struct.unpack_from("<i", data, 22)[0]
    pixels = data[offset:]
    rows = []
    for y in range(abs(height)):
        source = (abs(height) - 1 - y) if height > 0 else y
        row = pixels[source * width * 4 : (source + 1) * width * 4]
        out = bytearray()
        for x in range(width):
            b, g, r, _ = row[x * 4 : x * 4 + 4]
            out += bytes((r, g, b, 255))
        rows.append(bytes(out))
    return width, abs(height), b"".join(rows)


def write_png(path, width, height, rgba):
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw += rgba[y * width * 4 : (y + 1) * width * 4]

    def chunk(tag, payload):
        body = tag + payload
        return (
            struct.pack(">I", len(payload))
            + body
            + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    open(path, "wb").write(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def unpainted_row(width, height, rgba):
    """The first row the painter never covered, or None when there is none.

    round_corners below only touches ALPHA, so a black strip survives it
    looking like a deliberate border - which is how twenty black rows along
    the bottom of the shipped panel sat on the site unnoticed. The exe
    refuses to write one of these now; this is the second lock, and unlike
    the first it also catches a BMP that was already on disk.
    """
    for y in range(height):
        row = rgba[y * width * 4:(y + 1) * width * 4]
        if all(row[i] == 0 and row[i + 1] == 0 and row[i + 2] == 0
               for i in range(0, len(row), 4)):
            return y
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", default=EXE)
    parser.add_argument("--out", default=DOCS, help="directory to write panel-<lang>.png into")
    parser.add_argument("--count", default="3", help="how many invented accounts to show")
    args = parser.parse_args()

    if not os.path.exists(args.exe):
        raise SystemExit(f"no exe at {args.exe} - build it first (cargo build --release)")
    os.makedirs(args.out, exist_ok=True)
    work = os.path.join(ROOT, "target", "panel-shots")
    os.makedirs(work, exist_ok=True)

    env = dict(os.environ, DOSWITCH_UI_SCALE=SCALE)
    for lang in CASES:
        bmp = os.path.join(work, f"{lang}.bmp")
        done = subprocess.run(
            [args.exe, "--panel-snapshot", bmp, "--lang", lang, "--count", args.count],
            env=env,
            capture_output=True,
        )
        if done.returncode == 6:
            raise SystemExit(
                "another copy of the app is already showing a panel.\n"
                "close it and run again - its picture is not this build's picture."
            )
        if done.returncode != 0:
            raise SystemExit(f"panel snapshot failed for {lang}: exit {done.returncode}")
        width, height, rgba = read_bmp(bmp)
        blank = unpainted_row(width, height, rgba)
        if blank is not None:
            raise SystemExit(
                f"{lang}: row {blank} of {height} is pure black - the capture caught a "
                "strip the painter never covered. The panel clears to INK and paints "
                "over it, so no row it draws is black. Not shipping it: this is exactly "
                "how a black bottom border reached the site once already."
            )
        rgba = round_corners(width, height, rgba, CORNER_RADIUS)
        png = os.path.join(args.out, f"panel-{lang}.png")
        write_png(png, width, height, rgba)
        print(f"  {lang}: {width}x{height} -> {png}")

    print("\npanel screenshots written")
    return 0


if __name__ == "__main__":
    sys.exit(main())
