#!/usr/bin/env python3
"""Paired fixed-time Havindr capture: receiver depth and transparent ordering."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
visible, hidden = images['visible'], images['hidden']
difference = ImageChops.difference(visible, hidden)
def mean(box):
    return sum(ImageStat.Stat(difference.crop(box)).mean) / 3
# Interior robe pixels exclude silhouette edges and translucent spell glow.
robe = mean((484, 600, 495, 612))
ground = mean((500, 640, 555, 680))
# The brightest flame pixels are nearly opaque; translucent edges correctly
# blend over the changed ground and must not be required to remain identical.
core = []
for y in range(460, 555):
    for x in range(480, 575):
        r, g, b = hidden.getpixel((x, y))
        if r > 254 and g > 254 and b > 250:
            core.extend(difference.getpixel((x, y)))
flame = sum(core) / len(core) if core else float('inf')
# Keep the health bar outside the dark sample. A sub-byte mean tolerance
# permits independently drawn snow particles in the otherwise black ground.
dark = ImageChops.difference(images['dark_visible'], images['dark_hidden'])
dark_difference = sum(ImageStat.Stat(dark.crop((535, 630, 555, 655))).mean) / 3
dark_ground = sum(ImageStat.Stat(images['dark_visible'].crop((535, 630, 555, 655))).mean) / 3
result = {'passed': robe < .01 and ground > 30 and flame < 5 and len(core) > 180
          and dark_difference < 1 and dark_ground < 1,
          'dark_decal_difference': dark_difference, 'dark_ground_brightness': dark_ground,
          'robe_difference': robe, 'ground_difference': ground,
          'flame_core_difference': flame, 'flame_core_pixels': len(core) // 3,
          'native_visual_parity': 'unverified'}
(run / 'layering-pixels.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
if not result['passed']:
    raise SystemExit('Decal receiver depth / particle ordering regression')
