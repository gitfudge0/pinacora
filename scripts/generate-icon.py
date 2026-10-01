#!/usr/bin/env python3
"""Regenerate Reframed's original vector icon using macOS Core Graphics."""
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
RESOURCES = ROOT / "resources"
# Coordinates use the SVG's top-left origin; Swift flips the graphics context.
SHAPES = [
    (64, 64, 896, 896, 200, "#171c1b", None, 0),
    (222, 270, 532, 532, 62, "none", "#b98645", 34),
    (270, 222, 532, 532, 62, "#202725", "#f1e7ce", 38),
    (344, 296, 384, 384, 20, "none", "#8b9a8d", 8),
]
svg = ['<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">', '<title>Reframed app icon</title>']
for x, y, w, h, r, fill, stroke, sw in SHAPES:
    svg.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}" stroke="{stroke or "none"}" stroke-width="{sw}"/>')
svg.append('</svg>')
RESOURCES.joinpath('app-icon.svg').write_text('\n'.join(svg) + '\n')

def color(hex_value):
    return ', '.join(f'{label}: {int(hex_value[i:i + 2], 16) / 255}' for label, i in zip(('red', 'green', 'blue'), (1, 3, 5)))

swift = '''import AppKit
let output = CommandLine.arguments[1]
let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1024, pixelsHigh: 1024, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
let context = NSGraphicsContext(bitmapImageRep: bitmap)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = context
let cg = context.cgContext
cg.translateBy(x: 0, y: 1024)
cg.scaleBy(x: 1, y: -1)
'''
for x, y, w, h, r, fill, stroke, sw in SHAPES:
    swift += f'cg.addPath(CGPath(roundedRect: CGRect(x: {x}, y: {y}, width: {w}, height: {h}), cornerWidth: {r}, cornerHeight: {r}, transform: nil))\n'
    if fill != 'none':
        swift += f'cg.setFillColor(CGColor({color(fill)}, alpha: 1))\n'
    if stroke:
        swift += f'cg.setStrokeColor(CGColor({color(stroke)}, alpha: 1))\ncg.setLineWidth({sw})\n'
    mode = '.fillStroke' if fill != 'none' and stroke else '.fill' if fill != 'none' else '.stroke'
    swift += f'cg.drawPath(using: {mode})\n'
swift += '''NSGraphicsContext.restoreGraphicsState()
try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: output))
'''
with tempfile.TemporaryDirectory(prefix='reframed-icon-') as temporary:
    temp = pathlib.Path(temporary)
    source = temp / 'render.swift'
    source.write_text(swift)
    subprocess.run(['swift', str(source), str(RESOURCES / 'app-icon.png')], check=True)
    iconset = temp / 'AppIcon.iconset'
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            suffix = '@2x' if scale == 2 else ''
            subprocess.run(['sips', '-z', str(size * scale), str(size * scale), str(RESOURCES / 'app-icon.png'), '--out', str(iconset / f'icon_{size}x{size}{suffix}.png')], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(['iconutil', '-c', 'icns', str(iconset), '-o', str(RESOURCES / 'AppIcon.icns')], check=True)
