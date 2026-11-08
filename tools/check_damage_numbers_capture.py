#!/usr/bin/env python3
"""Paired FloatingTextManager capture: a pinned frame with the locked
direct-damage numbers drawn vs suppressed (real Fire spray on the demo goblin,
goblin melee on the Wizard). Native visual parity is unverified."""
import ast
import json
import re
import sys
from pathlib import Path
from PIL import Image, ImageChops

run = Path(sys.argv[1])
rows = json.loads((run / 'log.json').read_text())
images = {r['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB') for i, r in enumerate(rows)}
log = (run / 'viewer.log').read_text(errors='replace')
audits = re.findall(r'damage numbers audit: glyphs=(\[.*?\]) bounds=(\[.*?\]\]) colors=(\[.*?\]\])', log)
owners = re.findall(r'damage numbers owners: (\[.*?\])\n', log)
if not audits or not owners:
    raise SystemExit('no damage-number audit line')
glyphs, bounds, colors = (ast.literal_eval(v) for v in audits[-1])
owner_kinds = {kind for kind, _ in ast.literal_eval(owners[-1].replace('(', '[').replace(')', ']'))}
visible, hidden = images['numbers_visible'], images['numbers_hidden']
w, h = visible.size
diff = ImageChops.difference(visible, hidden).convert('L')
mask = Image.new('L', (w, h), 0)
boxes = [(max(int(b[0]) - 1, 0), max(int(b[1]) - 1, 0), min(int(b[2]) + 2, w), min(int(b[3]) + 2, h))
         for b in bounds]
for box in boxes:
    mask.paste(255, box)
d, m, v = diff.tobytes(), mask.tobytes(), visible.tobytes()
changed_inside = sum(1 for i in range(w * h) if m[i] and d[i] > 8)
outside_max = max((d[i] for i in range(w * h) if not m[i]), default=0)
strong = [i for i in range(w * h) if m[i] and d[i] > 40]
tinted = sum(1 for i in strong if v[3*i] > v[3*i+2] + 60 and v[3*i+1] > v[3*i+2] + 40)
cells_changed = all(any(d[y * w + x] > 40 for y in range(b[1], b[3]) for x in range(b[0], b[2]))
                    for b in boxes)
result = {
    'glyphs': glyphs,
    'number_cells': len(bounds),
    'owners': sorted(owner_kinds),
    'changed_pixels_inside': changed_inside,
    'every_cell_changed': cells_changed,
    'max_difference_outside': outside_max,
    'strong_pixels': len(strong),
    'yellow_tinted_fraction': tinted / len(strong) if strong else 0.0,
    'direct_number_default_colour': all(c[:3] == [1.0, 0.9, 0.0] for c in colors),
    'native_visual_parity': 'unverified',
}
result['passed'] = (changed_inside > 40 and cells_changed and outside_max <= 2
                    and result['yellow_tinted_fraction'] > 0.8
                    and result['direct_number_default_colour']
                    and owner_kinds == {'player', 'npc'}
                    and all(0 <= g <= 9 for g in glyphs))
(run / 'damage-numbers.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
if not result['passed']:
    raise SystemExit('damage number capture failed')
