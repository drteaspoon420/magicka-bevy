#!/usr/bin/env python3
"""Paired NPC attack check: exact travelling projectile hidden and restored."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
captures = {r['capture']: i + 1 for i, r in enumerate(rows)}
names = ['projectile_visible', 'model_hidden', 'trail_hidden', 'projectile_hidden', 'projectile_restored']
audits = [json.loads((run / f'npc-projectile-{captures[n]:02}.json').read_text()) for n in names]
a, model_hidden, trail_hidden, b, c = audits
assert a['owner'] and a['missile'], 'Missing NPC projectile identity'
assert a['travel'] > 2.5, 'Missile was not observed travelling'
assert a['particles'] and a['effects'], 'Missing retained travelling emitter and its particles'
for key in ['owner', 'missile', 'play_time', 'position', 'velocity', 'model', 'item_model', 'particles', 'effects']:
    assert all(a[key] == audit[key] for audit in audits), f'Unpinned simulation field {key}'
assert [a['hidden'], b['hidden'], c['hidden']] == [False, True, False]
if a['model'] or a['item_model']:
    assert a['mesh_descendants'] > 0, 'Projectile asset never produced a mesh'
if a['template'] == 'Goblin_bomber':
    assert any(surface['spec'][:2] == [0.5, 20.0] for surface in a['deferred_surfaces']), 'Bomb lost its authored specular surface'
images = [Image.open(run / f'{captures[n]:02}.png').convert('RGB') for n in names]
def compare(left, right):
    difference = ImageChops.difference(left, right)
    mask = difference.convert('L').point(lambda p: 255 if p > 12 else 0)
    return mask.histogram()[255], mask.getbbox(), difference
changed, box, difference = compare(images[0], images[3])
restored, _, _ = compare(images[0], images[4])
difference.save(run / 'npc-projectile-difference.png')
model_pixels, model_box, _ = compare(images[0], images[1])
trail_pixels, trail_box, _ = compare(images[0], images[2])
assert [audit['hide_model'] for audit in audits] == [False, True, False, True, False]
assert [audit['hide_particles'] for audit in audits] == [False, False, True, True, False]
if a['model'] or a['item_model']:
    assert model_pixels > 5, 'Missile model itself did not render; trail alone is insufficient'
else:
    assert model_pixels <= 10, 'Model control changed a particle-only projectile'
assert trail_pixels > 20, 'Missile particles themselves did not render'
report = {'passed': changed > 20 and restored <= max(10, changed // 50),
          'template': a['template'], 'missile': a['missile'], 'model': a['model'], 'item_model': a['item_model'],
          'travelling_particles': len(a['particles']), 'mesh_descendants': a['mesh_descendants'],
          'model_changed_pixels':model_pixels,'model_changed_box':model_box,
          'trail_changed_pixels':trail_pixels,'trail_changed_box':trail_box,
          'changed_pixels': changed, 'changed_box': box, 'restoration_changed_pixels': restored,
          'deferred_surfaces': a.get('deferred_surfaces', []),
          'native_visual_parity': 'unverified'}
(run / 'npc-projectile-check.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit('NPC travelling projectile did not produce a repeatable visible pixel difference')
