#!/usr/bin/env python3
"""Pinned spell/ArcaneBlade light on/off/restored capture; original visual parity unverified."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {r['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, r in enumerate(rows)}
on, off, restored = [images[k] for k in ('light_visible', 'light_hidden', 'light_restored')]
delta = ImageChops.difference(on, off)
changed = sum(max(p) > 12 for p in delta.getdata())
restore_max = max(max(p) for p in ImageChops.difference(on, restored).getdata())
report = {'changed_pixels_above_12': changed, 'changed_box': delta.getbbox(),
          'restored_max_error': restore_max, 'native_visual_parity': 'unverified',
          'passed': changed > 0 and restore_max == 0}
(run / ('spell-light-pixels.json' if len(sys.argv) > 2 and sys.argv[2] == 'spell' else 'capsule-light-pixels.json')).write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Spell light rendered regression')
