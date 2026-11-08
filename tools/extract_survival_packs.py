#!/usr/bin/env python3
"""Extract private reward pack configuration by static CIL decoding only.

No target code is loaded or executed. Default selection is packs with app ID 0;
--item-packs/--magick-packs explicitly select comma-separated native handles.
Each pack records its localization key, DLC app ID, PackThumbs cell and pool.
The extracted item names remain in output/.cache, never in source control.
"""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'output/evidence/tools'))
from il_disasm import Image
from dump_clr_metadata import main as metadata


def instructions(image, selector):
    methods = image.select([selector])
    if len(methods) != 1:
        raise ValueError(f'expected one method for {selector}')
    out = []
    for line in image.disasm(methods[0]):
        match = re.match(r'\s+IL_[0-9a-f]+: (\S+)\s*(.*)', line)
        if match:
            out.append(match.groups())
    return out


def integer(instruction):
    op, arg = instruction
    if op in ('ldc.i4', 'ldc.i4.s'):
        return int(arg)
    if op == 'ldc.i4.m1':
        return -1
    if op.startswith('ldc.i4.'):
        return int(op.rsplit('.', 1)[1])
    raise ValueError(f'expected integer, got {instruction}')


def extract(image):
    code = instructions(image, 'Magicka.DRM.HashTable::.cctor')
    arrays = {}
    for i, (op, arg) in enumerate(code):
        if op == 'stsfld' and i >= 2 and code[i - 1] == ('newarr', '[ext] System.String'):
            arrays[arg] = [None] * integer(code[i - 2])
        if op == 'stelem.ref' and i >= 3 and code[i - 3][0] == 'ldsfld' and code[i - 1][0] == 'ldstr':
            field = code[i - 3][1]
            if field in arrays:
                arrays[field][integer(code[i - 2])] = ast.literal_eval(code[i - 1][1])
    packs = {'items': [], 'magicks': []}
    code = instructions(image, 'Magicka.Levels.Packs.PackMan::.ctor')
    segment = []
    for instruction in code:
        op, arg = instruction
        segment.append(instruction)
        if op != 'newobj' or not any(f'Magicka.Levels.Packs.{kind}Pack::.ctor' in arg for kind in ('Item', 'Magick')):
            continue
        kind = 'items' if 'ItemPack::' in arg else 'magicks'
        # The constructor also loads the thumbnail texture; labels are keys.
        label = next(v for v in (ast.literal_eval(a) for o, a in segment if o == 'ldstr') if v.startswith('#'))
        if kind == 'items':
            getter = segment[-2][1].split(' [', 1)[0]
            field = next(a for o, a in instructions(image, getter) if o == 'ldsfld')
            values = arrays[field]
            if not values or any(v is None for v in values):
                raise ValueError(f'incomplete item pool {field}')
            values = [Path(v.replace('\\', '/')).stem.lower() for v in values]
            app = integer(segment[-3])
        else:
            start = next(i for i, (o, a) in enumerate(segment) if o == 'newarr' and a == 'Magicka.GameLogic.Spells.MagickType')
            values = [None] * integer(segment[start - 1])
            app = integer(segment[start - 2])
            for i in range(start + 1, len(segment)):
                if segment[i][0] == 'stelem.i4':
                    values[integer(segment[i - 2])] = integer(segment[i - 1])
            if not values or any(v is None for v in values):
                raise ValueError(f'incomplete magick pool {label}')
        # The UI/Menu/PackThumbs cell: the two integers before `new Point(x, y)`.
        point = next(i for i, (o, a) in enumerate(segment)
                     if o == 'newobj' and a.endswith('Microsoft.Xna.Framework.Point::.ctor'))
        thumb = [integer(segment[point - 2]), integer(segment[point - 1])]
        packs[kind].append(dict(handle=len(packs[kind]), name=label, app=app, thumb=thumb, values=values))
        segment = []
    if len(packs['items']) != 13 or len(packs['magicks']) != 6:
        raise ValueError('native pack layout changed; investigate before using')
    return packs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', type=Path, default=ROOT / 'target/Magicka/Magicka.exe')
    parser.add_argument('--output', type=Path, default=ROOT / 'output/.cache/survival-packs.json')
    parser.add_argument('--item-packs')
    parser.add_argument('--magick-packs')
    args = parser.parse_args()
    target = args.exe.resolve()
    raw = target.read_bytes()
    sha = hashlib.sha256(raw).hexdigest()
    md = ROOT / 'output/.cache/survival-pack-metadata' / sha
    if not (md / 'methods.tsv').is_file():
        metadata(str(target), str(md))
    packs = extract(Image(str(target), str(md)))
    for kind, selection in [('items', args.item_packs), ('magicks', args.magick_packs)]:
        handles = ({int(v) for v in selection.split(',') if v.strip()} if selection is not None
                   else {p['handle'] for p in packs[kind] if p['app'] == 0})
        if handles - {p['handle'] for p in packs[kind]}:
            raise ValueError(f'unknown {kind} handles')
        for pack in packs[kind]:
            pack['enabled'] = pack['handle'] in handles
    fingerprint = 0xcbf29ce484222325
    for byte in raw:
        fingerprint = ((fingerprint ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    record = dict(version=2, target_sha256=sha, target_fnv64=f'{fingerprint:016x}', **packs)
    # Derived target data belongs only in a private cache, never in target/.
    output = args.output.resolve()
    cache = (ROOT / 'output/.cache').resolve()
    if cache not in output.parents:
        raise ValueError('output must be under output/.cache')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(dict(output=str(output), sha256=sha,
                          enabled_items=sum(len(p['values']) for p in packs['items'] if p['enabled']),
                          enabled_magicks=sum(len(p['values']) for p in packs['magicks'] if p['enabled']))))


if __name__ == '__main__':
    main()
