#!/usr/bin/env python3
"""Paired pinned-time zap capture: the LightningHit flash (research/zap.md).

flash_visible and flash_hidden share simulation time and differ only by the
flash draw. The flash is additive (SrcAlpha/One) with LIGHTNINGCOLOR
(0.75, 0.5, 1.0), so it may only brighten pixels, mostly in blue then red,
and it must stay in a compact camera-facing region around the zapped goblin.
"""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
visible, hidden = images['flash_visible'], images['flash_hidden']
width, height = visible.size
vis, hid = visible.load(), hidden.load()
changed = []
darkened = 0
added = [0, 0, 0]
# Tint is judged where the hidden frame is not already bright: lightning
# bolts saturate blue first, so saturated pixels gain mostly red/green.
unsaturated = [0, 0, 0]
for y in range(height):
    for x in range(width):
        a, b = vis[x, y], hid[x, y]
        d = [a[k] - b[k] for k in range(3)]
        if max(abs(c) for c in d) > 8:
            changed.append((x, y))
            if min(d) < -8:
                darkened += 1
            for k in range(3):
                added[k] += d[k]
            if max(b) < 120:
                for k in range(3):
                    unsaturated[k] += d[k]
xs = [p[0] for p in changed]
ys = [p[1] for p in changed]
box = (min(xs), min(ys), max(xs), max(ys)) if changed else None
extent = max(box[2] - box[0], box[3] - box[1]) + 1 if box else 0
result = {
    'changed_pixels': len(changed),
    'darkened_pixels': darkened,
    'changed_box': box,
    'extent_px': extent,
    'added_rgb_sum': added,
    'unsaturated_added_rgb_sum': unsaturated,
    'native_visual_parity': 'unverified',
}
result['passed'] = bool(
    len(changed) > 300
    and darkened < 0.02 * len(changed)
    # Compact: one character-sized billboard, not a full-screen change.
    and 20 < extent < 260
    # LIGHTNINGCOLOR tint: blue > red > green in the added light.
    and unsaturated[2] > unsaturated[0] > unsaturated[1] > 0
)
(run / 'zap-flash-pixels.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
if not result['passed']:
    raise SystemExit('Zap LightningHit flash regression')
