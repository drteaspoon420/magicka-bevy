#!/usr/bin/env python3
"""Audit real Ice Force inputs and the actual viewer missile root transforms."""
import json
import math
import re
import sys
from pathlib import Path

run = Path(sys.argv[1])
rows = {r['capture']: r for r in json.loads((run / 'log.json').read_text())}
pattern = (r'missile flight audit: frame=(\d+) id=(\d+) model=(\w+) '
           r'position=(\[[^\n]*?\]) velocity=(\[[^\n]*?\]) rotation=(\[[^\n]*?\])')
records = []
for frame, ident, model, position, velocity, rotation in re.findall(pattern, (run / 'viewer.log').read_text()):
    if model != 'Ice3':
        continue
    v = json.loads(velocity)
    q = json.loads(rotation)
    speed = math.sqrt(sum(c*c for c in v))
    if speed == 0 or v[0] == v[2] == 0:
        continue
    x, y, z, w = q
    # Quaternion rotation of the mesh's native -Z forward axis.
    rendered_forward = [-2*(x*z+w*y), -2*(y*z-w*x), -(1-2*(x*x+y*y))]
    error = max(abs(a-b/speed) for a, b in zip(rendered_forward, v))
    records.append({'frame': int(frame), 'id': int(ident), 'position': json.loads(position),
                    'velocity': v, 'rotation': q, 'forward_error': error})
assert records, 'No rendered Ice3 shards from the real charged Force cast'
assert len({r['frame'] for r in records}) >= 3, 'Need flight across multiple updates'
assert all(r['forward_error'] < 1e-5 for r in records), 'Shard model does not face flight'
assert any(abs(r['rotation'][1]) > .1 for r in records), 'Nonidentity heading not exercised'
assert any(abs(r['velocity'][1]) > .1 for r in records), 'Flight pitch not exercised'
assert rows['charged']['state'] == 'Charge', 'Force charging input did not run'
report = {'passed': True, 'rendered_shards': len({r['id'] for r in records}),
          'sample_count': len(records), 'maximum_forward_error': max(r['forward_error'] for r in records),
          'native_visual_parity': 'unverified', 'records': records}
(run / 'missile-orientation-check.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps({k:v for k,v in report.items() if k != 'records'}, indent=2))
