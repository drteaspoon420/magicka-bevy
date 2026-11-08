#!/usr/bin/env python3
"""Audit the campaign motion fixture; report the unresolved leap warning."""
import ast
import json
from pathlib import Path
import re
import sys
from PIL import Image, ImageChops

root = Path(sys.argv[1])
log = root.with_suffix('.log').read_text()
pattern = re.compile(
    r'boss motion phase=(\d) state=(\w+) clip=Some\("([^"]+)"\) '
    r'fraction=([\d.]+) head=(\[[^\]]+\]) root=(\[[^\]]+\]) '
    r'visible=(true|false) effects=(\[[^\]]+\])'
)
phases = {}
for match in pattern.finditer(log):
    phase, state, clip, fraction, head, position, visible, effects = match.groups()
    phases[int(phase)] = dict(state=state, clip=clip, fraction=float(fraction),
                             head=ast.literal_eval(head), root=ast.literal_eval(position),
                             visible=visible == 'true', effects=effects)
assert set(phases) == set(range(5)), phases
assert not phases[0]['visible'] and phases[0]['fraction'] == 0
for phase, fraction in [(1, .25), (2, .85), (3, .35), (4, .95)]:
    item = phases[phase]
    assert item['visible'] and fraction <= item['fraction'] < fraction + .025, item
    assert item['clip'] == ('dive_attack' if phase < 3 else 'die_above'), item
assert phases[1]['head'][1] > phases[1]['root'][1] + 9.5
assert phases[2]['head'][1] < phases[1]['head'][1]
assert phases[3]['head'][1] > phases[4]['head'][1]
images = [Image.open(root / f'{i:02}.png').convert('RGB') for i in range(3, 8)]
assert all(im.size == (1280, 720) for im in images)

def changed(a, b, box):
    channels = ImageChops.difference(a.crop(box), b.crop(box)).split()
    maximum = ImageChops.lighter(ImageChops.lighter(*channels[:2]), channels[2])
    return sum(maximum.histogram()[25:])

# Fixed fixture regions: incoming head at left, rearing neck above the wizard.
leap_pixels = changed(images[0], images[1], (250, 220, 600, 410))
death_pixels = changed(images[3], images[4], (450, 140, 720, 370))
assert leap_pixels > 1000, leap_pixels
assert death_pixels > 1000, death_pixels
report = dict(phases=phases, leap_pose_changed_pixels=leap_pixels,
              death_pose_changed_pixels=death_pixels,
              leap_warning_active=phases[0]['effects'] != '[None, None, None]',
              original_visual_parity='unverified')
(root / 'motion-audit.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
