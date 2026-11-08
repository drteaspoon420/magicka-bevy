#!/usr/bin/env python3
"""Check the authored first/second dream and pinned apparition controls."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops


def count_difference(a, b, threshold):
    red, green, blue = ImageChops.difference(a, b).split()
    maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    return sum(maximum.histogram()[threshold + 1:])


def check(root):
    log = (root / "viewer.log").read_text()
    assert "Grimnir pinned venue=0 health=6" in log
    assert "Grimnir pinned venue=1 health=5" in log
    rows = json.loads((root / "log.json").read_text())
    frames = {
        row["capture"]: Image.open(root / f"{index:02}.png").convert("RGB")
        for index, row in enumerate(rows, 1)
    }
    a, b, c = [frames[name] for name in (
        "apparition_visible", "apparition_hidden", "apparition_restored"
    )]
    assert all(image.size == (1280, 720) for image in (a, b, c))
    bounds = (250, 50, 1180, 570)
    changed = count_difference(a.crop(bounds), b.crop(bounds), 4)
    restored = count_difference(a, c, 0)
    assert changed > 500, f"Apparition must visibly occupy the arena view: {changed}"
    assert restored == 0, f"Pinned restore differs: {restored}"
    second, second_hidden, second_restored = [frames[name] for name in (
        "second_venue", "second_hidden", "second_restored"
    )]
    second_changed = count_difference(second.crop(bounds), second_hidden.crop(bounds), 4)
    second_difference = count_difference(second, second_restored, 0)
    assert second_changed > 500, f"Apparition must follow the venue teleport: {second_changed}"
    assert second_difference == 0, f"Second venue restore differs: {second_difference}"
    report = {
        "bounds": bounds,
        "visible_hidden_changed_pixels": changed,
        "visible_restored_changed_pixels": restored,
        "second_visible_hidden_changed_pixels": second_changed,
        "second_visible_restored_changed_pixels": second_difference,
        "native_visual_parity": "unverified",
    }
    (root / "grimnir-measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    check(Path(sys.argv[1]))
