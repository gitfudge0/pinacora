#!/usr/bin/env python3
"""Package the selected Pinacora artwork as PNG and macOS ICNS icons.

Requires Pillow: python3 -m pip install Pillow
The checked-in source is preserved in full, including its ivory background.
"""
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    raise SystemExit("Pillow is required. Install it with: python3 -m pip install Pillow")

ROOT = Path(__file__).resolve().parents[1]
RESOURCES = ROOT / "resources"


def main():
    source = RESOURCES / "pinacora-icon-source.png"
    with Image.open(source) as artwork:
        if artwork.width != artwork.height:
            raise SystemExit(f"Icon source must be square; got {artwork.size}. No crop was applied.")
        icon = artwork.convert("RGBA").resize((1024, 1024), Image.Resampling.LANCZOS)
    icon.save(RESOURCES / "app-icon.png")
    icon.save(
        RESOURCES / "AppIcon.icns",
        format="ICNS",
        sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)],
    )
    print("Generated resources/app-icon.png and resources/AppIcon.icns")


if __name__ == "__main__":
    main()
