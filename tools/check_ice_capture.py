#!/usr/bin/env python3
"""Pixel regression for viewer-qa ice-mountaindale, with liquid inspection.
Checks rendering transitions only; does not establish original-game parity.
Requires Pillow. Leaves the result in the supplied private capture directory.
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
assert [row['capture'] for row in rows] == ['water', 'freeze', 'ice', 'thawed']
assert all(row['scene'] == 'md_s1' for row in rows)
images = [Image.open(run / f'{i:02}.png').convert('RGB') for i in (1, 3, 4)]
assert all(image.size == (1280, 720) for image in images)
water = (700, 520, 790, 555)
dry = (850, 150, 950, 200)


def difference(a, b, region):
    return sum(ImageStat.Stat(ImageChops.difference(a.crop(region), b.crop(region))).mean) / 3


liquid, ice, thawed = images
ice_mean = ImageStat.Stat(ice.crop(water)).mean
stats = {
    'water_to_ice': difference(liquid, ice, water),
    'ice_to_thawed': difference(ice, thawed, water),
    'dry_freeze_change': difference(liquid, ice, dry),
    'dry_thaw_change': difference(ice, thawed, dry),
    'ice_rgb': ice_mean,
}
checks = {
    'freeze_changes_surface': stats['water_to_ice'] > 20,
    'thaw_changes_surface': stats['ice_to_thawed'] > 20,
    'ice_is_bright_blue': ice_mean[2] > 180 and ice_mean[2] > ice_mean[0] + 20,
    'dry_geometry_stays_static': max(stats['dry_freeze_change'], stats['dry_thaw_change']) < 0.5,
}
report = {'checks': checks, 'stats': stats, 'original_game_parity': 'unverified'}
(run / 'ice-check.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
sys.exit(0 if all(checks.values()) else 1)
