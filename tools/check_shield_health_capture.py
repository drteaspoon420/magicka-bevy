#!/usr/bin/env python3
"""Check world/self bars in the real-input shield-bars capture. Requires Pillow."""
import json
import math
import re
import sys
from pathlib import Path
from PIL import Image

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
assert rows[-1]['capture'] == 'both_bars'
pattern = r'shield HUD audit: frame=(\d+) kind=(\w+) normalized=(\S+) bounds=(\[[^\n]*\])'
records = re.findall(pattern, (run / 'viewer.log').read_text())
assert records, 'No shield HUD submissions'
last_frame = max(int(r[0]) for r in records)
latest = {kind: (float(hp), json.loads(bounds)) for frame, kind, hp, bounds in records
          if int(frame) == last_frame}
assert 'Sphere' in latest and 'Self' in latest, 'Need world and self bars together'
image = Image.open(run / f'{len(rows):02}.png').convert('RGB')
checks = {}
for kind, (hp, bounds) in latest.items():
    assert 0 < hp < 1, 'Need nonempty partial shield fill'
    x0, y0, x1, y1 = bounds
    assert 0 <= x0 < x1 < image.width and 0 <= y0 < y1 < image.height, 'Bar outside viewport'
    box = (math.ceil(x0)+1, math.ceil(y0)+2, math.floor(x1)-1, math.floor(y1)-1)
    pixels = [image.getpixel((x, y)) for y in range(box[1], box[3])
              for x in range(box[0], box[2])]
    # The native HDR world tint can saturate to white in the GUI pass.
    # Check visible fill and a dark unfilled tail; exact color parity is open.
    bright = sum(r > 200 and g > 180 and b > 100 for r, g, b in pixels)
    tail_contrast = None
    if kind == 'Sphere':
        tail = [image.getpixel((x, math.floor((y0+y1)/2)))
                for x in range(math.ceil(x1)+4, math.ceil(x1)+12)]
        tail_contrast = sum(sum(c) for c in pixels)/len(pixels)/3 - sum(sum(c) for c in tail)/len(tail)/3
    checks[kind] = {'normalized': hp, 'bounds': bounds, 'bright_fill_pixels': bright,
                    'empty_tail_contrast': tail_contrast,
                    'passed': bright >= 8 and (tail_contrast is None or tail_contrast > 40)}
report = {'passed': all(v['passed'] for v in checks.values()), 'bars': checks,
          'native_visual_parity': 'unverified'}
(run / 'shield-health-check.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Shield HUD fill or unfilled tail missing from rendered atlas regions')
