#!/usr/bin/env python3
"""Frozen Earth+Shield native-strength/zero-overlay/restored render check."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {r['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, r in enumerate(rows)}
strong, weak, restored = [images[k] for k in ('strong', 'weak', 'restored')]
delta = ImageChops.difference(strong, weak)
pixels = delta.tobytes()
changed = sum(max(pixels[i:i+3]) > 12 for i in range(0, len(pixels), 3))
restore_max = max(ImageChops.difference(strong, restored).tobytes())
report = {'changed_pixels_above_12': changed, 'changed_box': delta.getbbox(),
          'restored_max_error': restore_max, 'native_visual_parity': 'unverified',
          'queued_elements': next(r['queue'] for r in rows if r['capture'] == 'queued4'),
          'passed': changed > 0 and restore_max == 0
                    and next(r['queue'] for r in rows if r['capture'] == 'queued4') == [128, 1, 1, 1, 1]}
(run / 'barrier-strength-pixels.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Barrier hardness overlay rendered regression')
