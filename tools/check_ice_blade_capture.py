#!/usr/bin/env python3
"""Paired pinned-time ice-blade capture (research/weapon-enchant.md).

blade_visible and blade_hidden share simulation time and differ only by the
Ice_Blade model drawn on the sword. The blade must change a compact,
blade-sized region and nothing else.
"""
import json
import sys
from pathlib import Path
from PIL import Image

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
visible, hidden = images['blade_visible'], images['blade_hidden']
width, height = visible.size
vis, hid = visible.load(), hidden.load()
changed = [(x, y) for y in range(height) for x in range(width)
           if max(abs(vis[x, y][k] - hid[x, y][k]) for k in range(3)) > 12]
xs = [p[0] for p in changed]
ys = [p[1] for p in changed]
box = (min(xs), min(ys), max(xs), max(ys)) if changed else None
extent = max(box[2] - box[0], box[3] - box[1]) + 1 if box else 0
result = {
    'changed_pixels': len(changed),
    'changed_box': box,
    'extent_px': extent,
    'native_visual_parity': 'unverified',
}
result['passed'] = bool(len(changed) > 150 and 20 < extent < 400)
(run / 'ice-blade-pixels.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
if not result['passed']:
    raise SystemExit('IceBlade model regression')
