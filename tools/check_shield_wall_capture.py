#!/usr/bin/env python3
"""Pinned live weapon wall visible/hidden/restored capture regression."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {r['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, r in enumerate(rows)}
on, off, restored = [images[k] for k in ('wall_visible', 'wall_hidden', 'wall_restored')]
delta = ImageChops.difference(on, off)
data = delta.tobytes()
changed = sum(max(data[i:i+3]) > 12 for i in range(0, len(data), 3))
restore_max = max(ImageChops.difference(on, restored).tobytes())
report = {'changed_pixels_above_12': changed, 'changed_box': delta.getbbox(),
          'restored_max_error': restore_max, 'native_visual_parity': 'unverified',
          'passed': changed > 0 and restore_max == 0}
(run / 'shield-wall-pixels.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Shield wall rendered regression')
