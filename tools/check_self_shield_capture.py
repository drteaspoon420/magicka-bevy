#!/usr/bin/env python3
"""Real E+self input: pure shield coating, animation and expiration."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
body = (465, 565, 516, 640)
dry = (250, 250, 350, 350)
def difference(a, b, box):
    return sum(ImageStat.Stat(ImageChops.difference(images[a], images[b]).crop(box)).mean) / 3

coating = difference('idle', 'shield', body)
animation = difference('shield', 'shield_later', body)
expiry = difference('idle', 'expired', body)
scenery = difference('idle', 'shield', dry)
log = (run / 'viewer.log').read_text()
checks = {
    'real_self_cast_creates_shield': 'autopilot shield: self-shield=Shield hp=' in log,
    'body_coating_visible': coating > 10,
    'coating_changes_over_time': animation > 2,
    'expiry_restores_body': expiry < 3 and 'autopilot expired: self-shield=None hp=' in log,
    'dry_scenery_preserved': scenery < 2,
}
report = {'passed': all(checks.values()), 'checks': checks,
          'body_difference': coating, 'animation_difference': animation,
          'expiry_difference': expiry, 'dry_difference': scenery,
          'native_visual_parity': 'unverified'}
(run / 'self-shield-pixels.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Self shield visual regression')
