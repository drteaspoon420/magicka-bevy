#!/usr/bin/env python3
"""Rewrite lava/rock/remelt regression; requires Pillow, no native parity claim."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
assert [row['capture'] for row in rows] == ['molten', 'freeze', 'rock', 'remelted']
assert all(row['scene'] == 'ew_s4' for row in rows)
images = [Image.open(run / f'{i:02}.png').convert('RGB') for i in (1, 3, 4)]
assert all(image.size == (1280, 720) for image in images)
surface = (620, 345, 680, 365)
dry = (200, 300, 350, 400)
means = [ImageStat.Stat(image.crop(surface)).mean for image in images]
dry_changes = [sum(ImageStat.Stat(ImageChops.difference(images[0].crop(dry), image.crop(dry))).mean) / 3 for image in images[1:]]
checks = {
    'molten_and_remelted_glow': all(c[0] > 150 and c[0] > c[2] + 80 for c in (means[0], means[2])),
    'rock_loses_red_heat': means[1][0] < min(means[0][0], means[2][0]) - 60,
    'rock_is_textured': max(ImageStat.Stat(images[1].crop(surface)).stddev) > 2,
    'dry_geometry_stays_static': max(dry_changes) < 0.5,
}
report = {'checks': checks, 'surface_rgb_molten_rock_remelted': means,
          'dry_changes': dry_changes, 'native_visual_parity': 'unverified',
          'freeze_input': 'harness fills retained vertex state; not a spell-input test'}
(run / 'lava-freeze-check.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
sys.exit(0 if all(checks.values()) else 1)
