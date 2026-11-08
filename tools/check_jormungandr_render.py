#!/usr/bin/env python3
"""Check the pinned Mountaindale boss visible/hidden/restored captures."""
import json
from pathlib import Path
import sys
from PIL import Image, ImageChops

root = Path(sys.argv[1])
images = [Image.open(root / f"{i:02}.png").convert("RGB") for i in (3, 4, 5)]
assert all(image.size == (1280, 720) for image in images)
# The emerged head and upper neck, excluding the overlapping wizard, staff
# particles, letterboxing and HUD. This belongs to the pinned authored camera.
bounds = (560, 145, 750, 375)
visible, hidden, restored = [image.crop(bounds) for image in images]


def changed_pixels(a, b, threshold):
    red, green, blue = ImageChops.difference(a, b).split()
    maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    return sum(maximum.histogram()[threshold + 1 :])


changed = changed_pixels(visible, hidden, 24)
restored_difference = changed_pixels(visible, restored, 2)
# A full body region must change; a few player/effect pixels cannot pass.
assert changed > 1000, changed
assert restored_difference < changed * 0.01, restored_difference
report = {
    "bounds": bounds,
    "visible_hidden_changed_pixels": changed,
    "visible_restored_changed_pixels": restored_difference,
}
(root / "boss-measurements.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
