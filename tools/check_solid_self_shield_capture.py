#!/usr/bin/env python3
"""Actual Earth, Ice, combined self-casts and removal: retained armour meshes."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
          for i, row in enumerate(rows)}
box = (462, 570, 519, 640)
def colors(name):
    image = images[name].crop(box)
    pixels = image.load()
    blue = brown = 0
    for y in range(image.height):
        for x in range(image.width):
            r, g, b = pixels[x,y]
            blue += b > r+30 and b > g+15
            brown += r > b+20 and g > b+10
    return {'blue': blue, 'brown': brown}
def diff(a, b, roi):
    return sum(ImageStat.Stat(ImageChops.difference(images[a],images[b]).crop(roi)).mean)/3

counts = {name:colors(name) for name in ['idle','earth','ice','combined','removed']}
log = (run / 'viewer.log').read_text()
expiry = diff('idle','removed',box)
dry = diff('idle','combined',(250,250,350,350))
checks = {
    'earth_shell': counts['earth']['brown'] > counts['idle']['brown']+400
        and 'autopilot earth: self-shield=Earth hp=' in log,
    'ice_shell': counts['ice']['blue'] > counts['idle']['blue']+700
        and 'autopilot ice: self-shield=Ice hp=' in log,
    'combined_shells': counts['combined']['blue'] > counts['idle']['blue']+400
        and counts['combined']['brown'] > counts['idle']['brown']+200
        and 'autopilot combined: self-shield=IcedEarth hp=' in log,
    'removed_shells': expiry < 3 and 'autopilot removed: self-shield=None hp=' in log,
    'scenery_preserved': dry < 2,
}
report = {'passed':all(checks.values()),'checks':checks,'body_colors':counts,
          'removed_difference':expiry,'dry_difference':dry,'native_visual_parity':'unverified'}
(run / 'solid-self-shield-pixels.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
if not report['passed']:
    raise SystemExit('Solid self shield visual regression')
