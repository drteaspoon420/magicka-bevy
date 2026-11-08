#!/usr/bin/env python3
"""Rendered lava regression for viewer-qa lava-endworld (liquid inspection).
Checks surface texture/glow/motion while sampled dry snow remains static.
Requires Pillow; no original-game visual parity claim.
"""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
assert [row['capture'] for row in rows] == ['liquid', 'flowing']
assert all(row['scene'] == 'ew_s4' for row in rows)
a, b = [Image.open(run / f'{i:02}.png').convert('RGB') for i in (1, 2)]
assert a.size == b.size == (1280, 720)
lava = (620, 345, 680, 365)
dry = (200, 300, 350, 400)
means = [ImageStat.Stat(image.crop(lava)).mean for image in (a, b)]
mean = means[0]
stats = {
    'lava_rgb': mean,
    'flowing_lava_rgb': means[1],
    'lava_green_stddev': ImageStat.Stat(a.crop(lava)).stddev[1],
    'lava_change': sum(ImageStat.Stat(ImageChops.difference(a.crop(lava), b.crop(lava))).mean) / 3,
    'dry_change': sum(ImageStat.Stat(ImageChops.difference(a.crop(dry), b.crop(dry))).mean) / 3,
}
checks = {
    # The moving temperature texture alternates red and yellow patches.
    # Require red heat in both frames and a yellow hot phase in either;
    # do not assume the first frame samples the hottest texture phase.
    'surface_glows': all(c[0] > 150 and c[0] > c[2] + 80 for c in means)
        and any(c[1] > 40 for c in means),
    'surface_is_textured': stats['lava_green_stddev'] > 5,
    'surface_moves': stats['lava_change'] > 1,
    'dry_geometry_stays_static': stats['dry_change'] < 0.5,
}
report = {'checks': checks, 'stats': stats, 'native_visual_parity': 'unverified'}
(run / 'lava-check.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
sys.exit(0 if all(checks.values()) else 1)
