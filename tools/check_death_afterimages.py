#!/usr/bin/env python3
"""Measure only Death's history toggle in a pinned, naturally moving pose."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


def changed_pixels(a, b, threshold):
    red, green, blue = ImageChops.difference(a, b).split()
    maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    return sum(maximum.histogram()[threshold + 1:])


def check(root):
    rows = json.loads((root / "log.json").read_text())
    frames = {
        row["capture"]: Image.open(root / f"{index:02}.png").convert("RGB")
        for index, row in enumerate(rows, 1)
    }
    visible, hidden, restored = [frames[name] for name in (
        "history_visible", "history_hidden", "history_restored"
    )]
    assert all(image.size == (1280, 720) for image in (visible, hidden, restored))
    # The audit centres the camera on Death and its oldest retained world pose.
    # Exclude the upper dialog region, letterboxing and lower-left spell HUD.
    bounds = (350, 180, 950, 570)
    changed = changed_pixels(visible.crop(bounds), hidden.crop(bounds), 4)
    restored_difference = changed_pixels(visible, restored, 0)
    assert changed > 100, f"History must visibly change the boss region: {changed}"
    assert restored_difference == 0, f"Pinned restore differs: {restored_difference}"
    report = {
        "bounds": bounds,
        "visible_hidden_changed_pixels": changed,
        "visible_restored_changed_pixels": restored_difference,
        "native_visual_parity": "unverified",
    }
    (root / "death-afterimage-measurements.json").write_text(
        json.dumps(report, indent=2) + "\n"
    )
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    check(Path(sys.argv[1]))
