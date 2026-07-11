#!/usr/bin/env python3
"""Crop full-screen VM captures to the portal region before committing.

Full-desktop frames are FORBIDDEN in committed evidence (they leak the operator
environment). This reads ``logs/crop_box.json`` (written by the driver) and crops
each ``screenshots/*-full.png`` to ``screenshots/<name>.png``, then deletes the
full-desktop source. Uses Pillow if present, else the ImageMagick ``convert`` CLI.
"""
from __future__ import annotations

import glob
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def load_box() -> dict:
    with open(os.path.join(HERE, "logs", "crop_box.json")) as f:
        return json.load(f)["crop_box"]


def crop_pillow(src: str, dst: str, box: dict) -> bool:
    try:
        from PIL import Image
    except ImportError:
        return False
    with Image.open(src) as im:
        w, h = im.size
        x0 = max(0, box["x"]); y0 = max(0, box["y"])
        x1 = min(w, box["x"] + box["w"]); y1 = min(h, box["y"] + box["h"])
        im.crop((x0, y0, x1, y1)).save(dst)
    return True


def crop_convert(src: str, dst: str, box: dict) -> bool:
    geom = f"{box['w']}x{box['h']}+{box['x']}+{box['y']}"
    r = subprocess.run(["convert", src, "-crop", geom, "+repage", dst],
                       capture_output=True)
    return r.returncode == 0


def main() -> int:
    box = load_box()
    shots = sorted(glob.glob(os.path.join(HERE, "screenshots", "*-full.png")))
    if not shots:
        print("no *-full.png to crop", file=sys.stderr)
        return 1
    for src in shots:
        name = os.path.basename(src).replace("-full.png", ".png")
        dst = os.path.join(HERE, "screenshots", name)
        ok = crop_pillow(src, dst, box) or crop_convert(src, dst, box)
        if not ok:
            print(f"FAILED to crop {src} (need Pillow or ImageMagick)", file=sys.stderr)
            return 2
        os.remove(src)
        print(f"cropped {os.path.basename(src)} -> {name} ({box['w']}x{box['h']})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
