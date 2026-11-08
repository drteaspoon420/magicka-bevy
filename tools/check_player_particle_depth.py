#!/usr/bin/env python3
"""Frozen Havindr wizard/item captures: particle occlusion and old-source control."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops, ImageStat


def main():
    run = Path(sys.argv[1])
    rows = json.loads((run / 'log.json').read_text())
    images = {row['capture']: Image.open(run / f'{i+1:02}.png').convert('RGB')
              for i, row in enumerate(rows)}
    frozen = [row for row in rows if row['capture'] != 'idle']
    for key in ('position', 'feet', 'camera_position', 'fov', 'health', 'animation'):
        assert all(row[key] == frozen[0][key] for row in frozen), ('unfrozen', key)
    baseline = images['baseline']
    differences = {name: ImageChops.difference(baseline, image)
                   for name, image in images.items()}
    # Opaque robe interior from the existing Havindr layering fixture. Bloom
    # is disabled in the harness so surrounding particles cannot brighten it.
    robe = (483, 594, 498, 612)
    means = {name: sum(ImageStat.Stat(differences[name].crop(robe)).mean) / 3
             for name in ('behind', 'old_source', 'front', 'restored')}
    # Removing the carried items gives their visible pixel mask. Use interior
    # mask pixels affected by both controls, avoiding antialiased silhouette
    # edges and pixels outside the fixture particle's footprint.
    item_difference = differences['no_items']
    item_mask = {(x, y) for y in range(555, 625) for x in range(455, 525)
                 if max(item_difference.getpixel((x, y))) > 30}
    interior = {p for p in item_mask
                if all((p[0]+dx, p[1]+dy) in item_mask
                       for dx, dy in ((-1, 0), (1, 0), (0, -1), (0, 1)))}
    tested_items = [p for p in interior
                    if min(max(differences[name].getpixel(p))
                           for name in ('old_source', 'front')) > 80]
    item_max = max((max(differences['behind'].getpixel(p))
                    for p in tested_items), default=255)
    restored_max = max(high for _, high in differences['restored'].getextrema())
    passed = (means['behind'] < .01 and means['old_source'] > 80
              and means['front'] > 80 and restored_max == 0
              and len(tested_items) >= 3 and item_max == 0)
    report = {'passed': passed, 'robe_difference': means,
              'tested_equipment_pixels': len(tested_items),
              'equipment_behind_max_difference': item_max,
              'restored_whole_frame_max_difference': restored_max,
              'bloom': 'disabled for direct-occlusion measurement',
              'native_visual_parity': 'unverified'}
    (run / 'particle-depth-pixels.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    if not passed:
        raise SystemExit('Live player/equipment particle depth regression')


if __name__ == '__main__':
    main()
