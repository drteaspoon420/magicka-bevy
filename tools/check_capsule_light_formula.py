#!/usr/bin/env python3
"""Compare capsule-light algebra with original preshader and pixel bytecode.

Usage: python3 output/rewrite/tools/check_capsule_light_formula.py EFFECT.fxo [RESULT.json]
This executes a static bytecode interpreter, never the retail executable.
"""
import json
import math
from pathlib import Path
import random
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'evidence' / 'tools'))
from d3d9_disasm import blob_end
from fx_preshader import ctab, preshaders, run_pres
from ps3_interp import Shader


def dot(a, b):
    return sum(x*y for x, y in zip(a, b))


def norm(a):
    length = math.sqrt(dot(a, a))
    return [x / length for x in a]


def main():
    data = Path(sys.argv[1]).read_bytes()
    shader = Shader(data[0x908:blob_end(data, 0x908)])
    pres = next(words for off, words in preshaders(data) if off == 0xaa0)
    names = {}
    i = 1
    while i < len(pres) and pres[i] & 0xffff == 0xfffe:
        size = pres[i] >> 16
        if struct.pack('<I', pres[i+1]) == b'CTAB':
            names = ctab(pres[i+2:i+1+size])
        i += size + 1
    parameters = {name: k for k, name in names.items()}
    rng = random.Random(20261009)
    maximum = 0.0
    regions = {'start': 0, 'segment': 0, 'end': 0, 'outside_radius': 0}
    for _ in range(4096):
        start = [rng.uniform(-4, 4) for _ in range(3)]
        segment = [rng.uniform(-5, 5) for _ in range(3)]
        end = [a+b for a, b in zip(start, segment)]
        position = [rng.uniform(-8, 8) for _ in range(3)]
        normal = norm([rng.uniform(-1, 1) for _ in range(3)])
        radius = rng.uniform(.25, 8)
        diffuse = [rng.uniform(0, 4) for _ in range(3)]
        ambient = [rng.uniform(0, 1) for _ in range(3)]
        inputs = {}
        for name, values in [('Start', start), ('End', end), ('Radius', [radius])]:
            inputs.update({parameters[name]+k: v for k, v in enumerate(values)})
        output = run_pres(pres, inputs)
        constants = {k//4: tuple(output.get(k//4*4+j, 0) for j in range(4))
                     for k in output}
        constants.update({4: (*start, 0), 5: (*diffuse, 0), 6: (*ambient, 0),
                          7: (*position, 0), 8: (0, 0, 0, 0)})
        normal_texel = tuple((c+1)*.5 for c in normal) + (0,)
        actual = shader.run({'v0': (0, 0, 0, 1), 'v1': (0, 0, 0, 0)},
                            constants, {},
                            lambda s, _: (1, 0, 0, 0) if s == 0 else normal_texel)['oC0']
        projection = dot([p-s for p, s in zip(position, start)], segment) / dot(segment, segment)
        regions['start' if projection < 0 else 'end' if projection > 1 else 'segment'] += 1
        t = min(max(projection, 0), 1)
        closest = [s+t*d for s, d in zip(start, segment)]
        offset = [c-p for c, p in zip(closest, position)]
        distance = math.sqrt(dot(offset, offset))
        regions['outside_radius'] += distance >= radius
        fall = min(max(1-distance/radius, 0), 1)
        lambert = min(max(dot(normal, norm(offset)), 0), 1)
        expected = [1-2**(-fall*(lambert*d+a)) for d, a in zip(diffuse, ambient)]
        maximum = max(maximum, max(abs(a-b) for a, b in zip(actual[:3], expected)))
        assert actual[3] == 0, ('unexpected specular', actual)
    assert all(regions.values()), regions
    # Native preshader constants use float32; the closed-form reference uses doubles.
    assert maximum < 1e-5, maximum
    report = {'status': 'PASS', 'cases': 4096, 'regions': regions,
              'max_visible_output_error': maximum,
              'source': 'CapsuleLight ps_3_0 @0x908, preshader @0xaa0',
              'excluded_singularities': 'zero-length segment; surface exactly on axis'}
    if len(sys.argv) > 2:
        Path(sys.argv[2]).write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
