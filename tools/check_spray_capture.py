#!/usr/bin/env python3
"""Real-input Havindr spray mark regression; original parity unverified."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run, scenario = Path(sys.argv[1]), sys.argv[2]
cell = {'spray-fire':20, 'spray-cold':22, 'spray-water':23}[scenario]
label = 'faded' if scenario == 'spray-fire' else 'marks'
rows = json.loads((run/'log.json').read_text())
images = {r['capture']:Image.open(run/f'{i+1:02}.png').convert('RGB') for i,r in enumerate(rows)}
def diff(box):
    return sum(ImageStat.Stat(ImageChops.difference(images['idle'].crop(box),images[label].crop(box))).mean)/3
mark = diff((480,465,565,567))
dry = diff((200,200,300,300))
expected = f'autopilot {label}: decal cells={[cell]*4}' in (run/'viewer.log').read_text()
result = {'passed':mark>3 and dry<1 and expected, 'scenario':scenario,
          'cell':cell,'mark_difference':mark,'dry_difference':dry,
          'four_retained_marks':expected,'native_visual_parity':'unverified'}
(run/'spray-pixels.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
if not result['passed']: raise SystemExit('Spray mark capture failed')
