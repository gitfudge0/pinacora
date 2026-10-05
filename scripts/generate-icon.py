#!/usr/bin/env python3
"""Extract the colored Pinacora artwork and package PNG and macOS ICNS icons.

Requires Pillow: python3 -m pip install Pillow
The original square source is preserved. Ivory exterior and internal gaps become
transparent; colored artwork keeps its original geometry and padding.
"""
from pathlib import Path

try:
    from PIL import Image, ImageFilter
except ImportError:
    raise SystemExit("Pillow is required. Install it with: python3 -m pip install Pillow")

ROOT = Path(__file__).resolve().parents[1]
RESOURCES = ROOT / "resources"
BACKGROUND = (254, 249, 238)


def extract_artwork(artwork):
    """Remove ivory and unmatte blended edge pixels against nearby solid color."""
    source = artwork.convert("RGB")
    pixels = source.load()
    # The ivory texture has very little chroma. Erosion leaves dependable
    # foreground samples, avoiding partially blended pixels at the boundary.
    mask = Image.new("L", source.size)
    mask.putdata([
        255 if max(rgb) - min(rgb) > 45 and min(rgb) < 210 else 0
        for rgb in source.getdata()
    ])
    solid = mask.filter(ImageFilter.MinFilter(5)).load()
    result = Image.new("RGBA", source.size)
    output = result.load()
    width, height = source.size
    for y in range(height):
        for x in range(width):
            color = pixels[x, y]
            if solid[x, y]:
                output[x, y] = (*color, 255)
                continue
            if max(color) - min(color) <= 25:
                continue
            foreground = None
            for radius in range(1, 7):
                candidates = []
                for dy in range(-radius, radius + 1):
                    for dx in range(-radius, radius + 1):
                        if max(abs(dx), abs(dy)) != radius:
                            continue
                        nx, ny = x + dx, y + dy
                        if 0 <= nx < width and 0 <= ny < height and solid[nx, ny]:
                            candidates.append((dx * dx + dy * dy, pixels[nx, ny]))
                if candidates:
                    foreground = min(candidates, key=lambda item: item[0])[1]
                    break
            if foreground is None:
                continue
            direction = tuple(f - b for f, b in zip(foreground, BACKGROUND))
            coverage = sum((c - b) * d for c, b, d in zip(color, BACKGROUND, direction))
            coverage /= sum(d * d for d in direction)
            coverage = max(0.0, min(1.0, coverage))
            if coverage < 0.02:
                continue
            # Undo the ivory blend, so translucent edges have foreground RGB
            # rather than a pale halo when composited on a dark macOS Dock.
            unmatted = tuple(
                round(max(0, min(255, b + (c - b) / coverage)))
                for c, b in zip(color, BACKGROUND)
            )
            output[x, y] = (*unmatted, round(coverage * 255))
    return result


def main():
    source = RESOURCES / "pinacora-icon-source.png"
    with Image.open(source) as artwork:
        if artwork.width != artwork.height:
            raise SystemExit(f"Icon source must be square; got {artwork.size}. No crop was applied.")
        icon = extract_artwork(artwork).resize((1024, 1024), Image.Resampling.LANCZOS)
    icon.save(RESOURCES / "app-icon.png")
    icon.save(
        RESOURCES / "AppIcon.icns",
        format="ICNS",
        sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)],
    )
    print("Generated resources/app-icon.png and resources/AppIcon.icns")


if __name__ == "__main__":
    main()
