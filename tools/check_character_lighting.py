#!/usr/bin/env python3
"""Check the frozen Havindr character-lighting bright/dark/restored captures."""
import json
from pathlib import Path
import sys
from PIL import Image, ImageStat

root = Path(sys.argv[1])
images = [Image.open(root / f"{i:02}.png").convert("RGB") for i in (3, 4, 5)]
assert all(image.size == (1280, 720) for image in images)
# Body interiors in this authored camera fixture, excluding HUD, floating
# numbers, held staff glow and NPC health bars.
regions = {"player": (461, 594, 497, 636), "npc": (539, 600, 580, 636)}
report = {}
for name, bounds in regions.items():
    means = [sum(ImageStat.Stat(image.crop(bounds)).mean) / 3 for image in images]
    bright, dark, restored = means
    assert bright > 5 and dark < bright * 0.25, (name, means)
    assert abs(restored - bright) < bright * 0.1, (name, means)
    report[name] = {"bounds": bounds, "mean_rgb": means}
(root / "lighting-measurements.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
