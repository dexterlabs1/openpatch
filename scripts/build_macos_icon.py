#!/usr/bin/env python3
"""Regenerate the PNG and ICNS assets from the original SVG on macOS."""
from pathlib import Path
import subprocess
import tempfile

assets = Path(__file__).resolve().parents[1] / "crates/openpatch-desktop/assets"
source = assets / "openpatch.svg"
png = assets / "openpatch.png"
subprocess.run(["sips", "-s", "format", "png", str(source), "--out", str(png)], check=True)
with tempfile.TemporaryDirectory(prefix="openpatch-icon-") as directory:
    iconset = Path(directory) / "OpenPatch.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            pixels = size * scale
            suffix = "@2x" if scale == 2 else ""
            target = iconset / f"icon_{size}x{size}{suffix}.png"
            subprocess.run(["sips", "-z", str(pixels), str(pixels), str(png), "--out", str(target)], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(assets / "OpenPatch.icns")], check=True)
print(assets / "OpenPatch.icns")
