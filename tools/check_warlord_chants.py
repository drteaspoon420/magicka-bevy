#!/usr/bin/env python3
"""Check the shield-anchored fire/empty/arcane Warlord capture controls."""
import json
from pathlib import Path
import sys
from PIL import Image, ImageChops

root = Path(sys.argv[1])
log = root.with_suffix('.log').read_text()
for state in ['act=fire queue=Elements(8) fire=1 arcane=0',
              'act=empty queue=Elements(0) fire=0 arcane=0',
              'act=arcane queue=Elements(32) fire=0 arcane=1']:
    assert state in log, state
images = [Image.open(root / f'{i:02}.png').convert('RGB') for i in (3, 4, 5)]
assert all(image.size == (1280, 720) for image in images)
# Shield effect0 at its lower-right edge; exclude room torches and the HUD.
bounds = (682, 403, 710, 432)
fire, empty, arcane = [image.crop(bounds) for image in images]

def changed(a, b):
    r, g, blue = ImageChops.difference(a, b).split()
    maximum = ImageChops.lighter(ImageChops.lighter(r, g), blue)
    return sum(maximum.histogram()[25:])

fire_pixels = changed(fire, empty)
arcane_pixels = changed(arcane, empty)
color_pixels = changed(fire, arcane)
assert min(fire_pixels, arcane_pixels) > 12, (fire_pixels, arcane_pixels)
assert color_pixels > 10, color_pixels
report = dict(bounds=bounds, fire_empty_changed_pixels=fire_pixels,
              arcane_empty_changed_pixels=arcane_pixels,
              fire_arcane_changed_pixels=color_pixels,
              original_visual_parity='unverified')
(root / 'chant-measurements.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
