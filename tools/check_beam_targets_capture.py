#!/usr/bin/env python3
"""Check seeded beam target state and repeatable pixels in each ray's span."""
import json
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
files = {r['capture']: run / f'{i+1:02}.png' for i, r in enumerate(rows)}
phases = {i: json.loads((run / f'beam-target-{i}.json').read_text()) for i in (2, 3, 4, 7)}
a, b, c = [phases[i] for i in (2, 3, 4)]
assert all(p.get('cast_input') and p.get('active_spell') for p in (a,b,c)), 'Beam did not come from Cast'
queued = next(r for r in rows if r['capture'] == 'queued')
assert queued['queue'] == ([32] if a['mode'] == 0 else [32,8]), 'Element keys did not queue the spell'
for key in ('mode', 'time', 'root', 'target', 'rails', 'npcs', 'view', 'projection'):
    assert a[key] == b[key] == c[key], f'Simulation not pinned: {key}'
assert a['render_beams'] > 0 and b['render_beams'] == 0 and c['render_beams'] == a['render_beams']
assert not phases[7]['rails'] and not phases[7]['render_beams'], 'Beam did not expire'
if a['mode'] == 0:
    assert a['portal_connected']
    assert len(a['rails']) == 2, a['rails']
    assert any(r['depth'] == 1 and r['out_portal'] is not None for r in a['rails'])
    loss = a['before_hp'] - next(n['hp'] for n in a['npcs'] if n['id'] == a['target'])
    ticks = loss / 56.25  # Native Arcane 225, beam tick 0.25, Goblin_miner resistance 1.
    assert ticks >= 1 and abs(ticks-round(ticks)) < 0.001, f'Unexpected target damage: {loss}'
    root = next(r for r in a['rails'] if r['depth'] == 0)
    child = next(r for r in a['rails'] if r['depth'] == 1)
    assert root['child'] == child['id'] and root['direction'] == child['direction']
    expected = [a['portal_a'][i] + root['position'][i] + root['direction'][i]*(root['length']-0.5) - a['portal_b'][i] for i in range(3)]
    assert all(abs(x-y) < 0.0001 for x,y in zip(expected, child['position'])), 'Wrong exit offset'
    assert not phases[7]['portal_connected']
else:
    assert any(n['template'] == 'elemental_fire' for n in a['npcs']), 'Egg did not hatch Fire'

def transform(v, m):
    return [sum(v[j] * m[j*4+i] for j in range(4)) for i in range(4)]

def project(p):
    clip = transform(transform([*p, 1], a['view']), a['projection'])
    assert clip[3] > 0
    return ((clip[0]/clip[3]+1)*640, (1-clip[1]/clip[3])*360)

def compare(left, right):
    d = ImageChops.difference(left, right).convert('L')
    return [(i % d.width, i // d.width) for i, value in enumerate(d.tobytes()) if value > 12]

visible, hidden, restored = [Image.open(files[n]).convert('RGB') for n in ('visible', 'hidden', 'restored')]
changed = compare(visible, hidden)
restore = compare(visible, restored)
assert len(changed) > 20, 'Rendered beam made no visible difference'
assert len(restore) <= max(10, len(changed)//50), 'Restored beam is not repeatable'
spans = []
for rail in a['rails']:
    p, d, length = rail['position'], rail['direction'], rail['length']
    start, end = [project([p[i] + d[i]*length*t for i in range(3)]) for t in (0.2, 0.8)]
    dx, dy = end[0]-start[0], end[1]-start[1]
    denominator = dx*dx + dy*dy
    assert denominator > 1
    count = 0
    for x, y in changed:
        t = ((x-start[0])*dx+(y-start[1])*dy)/denominator
        if 0 <= t <= 1 and (x-start[0]-t*dx)**2+(y-start[1]-t*dy)**2 <= 12**2:
            count += 1
    spans.append({'rail': rail['id'], 'depth': rail['depth'], 'screen_span': [start,end], 'changed_pixels': count})
    assert count > 15, f'Rail span did not render: {spans[-1]}'
report = {'passed': True, 'mode': a['mode'], 'changed_pixels': len(changed),
          'restoration_changed_pixels': len(restore), 'spans': spans,
          'native_visual_parity': 'unverified', 'fixture': 'targets seeded with runtime APIs; ordinary element keys, Cast and animation create the beam'}
(run / 'beam-target-check.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2))
