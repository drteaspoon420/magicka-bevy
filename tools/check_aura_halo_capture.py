#!/usr/bin/env python3
"""Fire aura's original elevated halo; ground decals lie below this ROI."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat
run = Path(sys.argv[1])
rows = json.loads((run/'log.json').read_text())
images = {r['capture']:Image.open(run/f'{i+1:02}.png').convert('RGB') for i,r in enumerate(rows)}
box = (445,535,540,572)
def red(name):
    im = images[name].crop(box)
    p = im.load()
    return sum(p[x,y][0] > p[x,y][2]+40 and p[x,y][0] > p[x,y][1]+20
               for y in range(im.height) for x in range(im.width))
def diff(a,b,roi):
    return sum(ImageStat.Stat(ImageChops.difference(images[a],images[b]).crop(roi)).mean)/3
counts = {n:red(n) for n in ['idle','aura','aura_later','aura_expired']}
restored = diff('idle','aura_expired',box)
dry = diff('idle','aura',(250,250,350,350))
checks = {'fire_halo_visible':counts['aura'] > counts['idle']+100,
          'halo_retained':counts['aura_later'] > counts['idle']+100,
          'expiry_restores_view':restored < 2 and counts['aura_expired'] < counts['idle']+10,
          'scenery_preserved':dry < 2}
report = {'passed':all(checks.values()),'checks':checks,'red_pixels':counts,
          'expiry_difference':restored,'dry_difference':dry,'native_visual_parity':'unverified'}
(run/'halo-pixels.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
if not report['passed']: raise SystemExit('Aura halo visual regression')
