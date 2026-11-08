#!/usr/bin/env python3
"""Paired pinned-time Ice Area capture (research/ground-area.md, IceSpikes).

spikes_visible and spikes_hidden share simulation time and differ only by
the IceSpikes models. The spikes stand on two rings around the blast, so the
change must cover a ring-sized region and be spread across it.
"""
import json
import sys
from pathlib import Path
from PIL import Image

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
log = (run / 'viewer.log').read_text()
spikes_line = next((l for l in log.splitlines()
                    if 'autopilot spikes_visible: ice spikes' in l), '')
visible, hidden = images['spikes_visible'], images['spikes_hidden']
width, height = visible.size
vis, hid = visible.load(), hidden.load()
changed = [(x, y) for y in range(height) for x in range(width)
           if max(abs(vis[x, y][k] - hid[x, y][k]) for k in range(3)) > 12]
xs = [p[0] for p in changed]
ys = [p[1] for p in changed]
box = (min(xs), min(ys), max(xs), max(ys)) if changed else None
# Spread: changed pixels fall in at least 6 of the 8 octants around the
# changed region's centre (a ring, not one blob).
octants = set()
if box:
    cx, cy = (box[0] + box[2]) / 2, (box[1] + box[3]) / 2
    import math
    for x, y in changed:
        if abs(x - cx) + abs(y - cy) > 10:
            octants.add(int((math.atan2(y - cy, x - cx) + math.pi) / (math.pi / 4)) % 8)
result = {
    'log': spikes_line.split('autopilot ')[-1],
    'changed_pixels': len(changed),
    'changed_box': box,
    'octants': len(octants),
    'native_visual_parity': 'unverified',
}
result['passed'] = bool(len(changed) > 500 and len(octants) >= 6 and '[]' not in spikes_line
                        and spikes_line != '')
(run / 'ice-spikes-pixels.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
if not result['passed']:
    raise SystemExit('IceSpikes regression')
