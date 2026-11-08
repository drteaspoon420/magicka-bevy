#!/usr/bin/env python3
"""Headless GPU outline selection regression, using procedural assets only."""
import json
from pathlib import Path
import subprocess
import sys
from PIL import Image

viewer = str(Path(sys.argv[1]).resolve())
out = Path(sys.argv[2]).resolve()
out.mkdir(parents=True, exist_ok=True)
report = {}
for samples in (1, 4):
    capture = out / f'occluders-{samples}x.png'
    args = [viewer, '--outline-check', str(capture)]
    if samples == 4:
        args.append('--msaa4')
    run = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=90)
    (out / f'occluders-{samples}x.log').write_text(run.stdout)
    if run.returncode or 'read-only file system' in run.stdout.lower() or 'Validation Error' in run.stdout:
        raise RuntimeError(run.stdout[-6000:])
    image = Image.open(capture).convert('RGB')
    values = [sum(g >= 100 and r <= 8 and b <= 8 for r, g, b in image.crop((i*256, 0, (i+1)*256, 256)).get_flattened_data()) for i in range(6)]
    assert values[0] > 100 and values[2] > 100 and values[1] == values[3] == values[4] == values[5] == 0, values
    report[f'{samples}x'] = dict(zip(('level_lit', 'entity_lit', 'level_terrain', 'entity_terrain', 'entity_standard', 'entity_over_level'), values))
(out / 'occluder-report.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
