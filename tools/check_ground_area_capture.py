#!/usr/bin/env python3
"""Actual S+D area-cast flash and retained cracks in Havindr."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
box = (350, 510, 455, 675)
flash = ImageStat.Stat(images['flash'].crop(box)).mean
faded = ImageStat.Stat(images['faded'].crop(box)).mean
delta = [a-b for a,b in zip(flash, faded)]
log = (run / 'viewer.log').read_text()
checks = {
    'colored_flash_lights_scenery': delta[0] > 15 and delta[0] > delta[2] + 10,
    'sixteen_ground_effects': 'autopilot flash: ground-earth-effects=16 lights=[(' in log,
    'effects_and_light_end': 'autopilot faded: ground-earth-effects=0 lights=[]' in log,
    'retained_cracks': f'autopilot faded: decal cells={[35]*16}' in log,
}
report = {'passed': all(checks.values()), 'checks': checks,
          'flash_rgb': flash, 'faded_rgb': faded, 'flash_difference': delta,
          'native_visual_parity': 'unverified'}
(run / 'ground-area-pixels.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('Ground area visual regression')
