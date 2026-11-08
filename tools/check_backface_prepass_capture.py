#!/usr/bin/env python3
"""viewer-qa ruins-backface: the player at the reported ru_s4 spot (19, -19).
A level wall seen from behind used to write prepass depth while the main pass
culled it, leaving a pure-black (clear colour) occluder over the floor and
player. Require the floor around the player to be drawn (not clear colour).
Requires Pillow; no original-game visual parity claim.
"""
import json
import sys
from pathlib import Path
from PIL import Image

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
assert all(row['scene'] == 'ru_s4' for row in rows)
image = Image.open(run / 'final.png').convert('RGB')
# A band around the screen centre (the camera follows the player).
box = (560, 260, 880, 540)
crop = image.crop(box)
pixels = [crop.getpixel((x, y)) for y in range(crop.height) for x in range(crop.width)]
black = sum(1 for p in pixels if p == (0, 0, 0)) / len(pixels)
checks = {'floor_not_clear_colour': black < 0.05}
report = {'checks': checks, 'stats': {'black_fraction': black}, 'native_visual_parity': 'unverified'}
(run / 'backface-check.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not all(checks.values()):
    raise SystemExit('back-face prepass occluder present')
