#!/usr/bin/env python3
"""Verify Vlad's authored retained-history toggle and exact restoration."""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "evidence/tools"))
from png_diff import read_png, diff


def crop(image, bounds):
    _, _, channels, rows = image
    left, top, right, bottom = bounds
    return right - left, bottom - top, channels, [
        row[left * channels:right * channels] for row in rows[top:bottom]
    ]


def check(root):
    log = (root / "viewer.log").read_text()
    for phase in (1, 2, 3):
        assert f"Vlad afterimages phase={phase} poses=5" in log, "Missing populated Vlad history audit"
    rows = json.loads((root / "log.json").read_text())
    frames = {
        row["capture"]: read_png(root / f"{index:02}.png")
        for index, row in enumerate(rows, 1)
    }
    visible, hidden, restored = [frames[name] for name in ("visible", "hidden", "restored")]
    assert all(frame[:2] == (1280, 720) for frame in (visible, hidden, restored))
    bounds = (350, 180, 950, 570)
    changed = diff(crop(visible, bounds), crop(hidden, bounds), 4)[1]
    restored_difference = diff(visible, restored, 0)[1]
    assert changed > 100, f"Vlad's trail must change the boss region: {changed}"
    assert restored_difference == 0, f"Pinned restore differs: {restored_difference}"
    report = {
        "bounds": bounds,
        "visible_hidden_changed_pixels": changed,
        "visible_restored_changed_pixels": restored_difference,
        "native_visual_parity": "unverified",
    }
    (root / "vlad-afterimage-measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    check(Path(sys.argv[1]))
