#!/usr/bin/env python3
"""Pixel regressions for the named Havindr atlas fixture, not native parity."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
captures = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
            for i, row in enumerate(rows)}
ground, marks, expired = [captures[k] for k in ('ground', 'decals', 'expired')]
def difference(a, b, box):
    return sum(ImageStat.Stat(ImageChops.difference(a.crop(box), b.crop(box))).mean) / 3

cells = [20, 22, 23, 0, 4, 8, 12, 16]
results = []
for i, cell in enumerate(cells):
    x, y = [447, 544][i % 2], [338, 398, 458, 518][i // 2]
    box = (x-35, y-22, x+35, y+22)
    visible = difference(ground, marks, box)
    restored = difference(ground, expired, box)
    results.append({'cell': cell, 'box': box, 'mark_difference': visible,
                    'expired_difference': restored, 'passed': visible > 2 and restored < 1})
dry = difference(ground, marks, (200, 200, 300, 300))
log = (run / 'viewer.log').read_text()
state = 'autopilot decals: decal cells=[20, 22, 23, 0, 4, 8, 12, 16]' in log and 'autopilot expired: decal cells=[]' in log
report = {'passed': all(r['passed'] for r in results) and dry < 1 and state,
          'cells': results, 'dry_difference': dry, 'expected_state': state,
          'native_visual_parity': 'unverified'}
(run / 'decal-pixels.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Decal capture regression failed')
