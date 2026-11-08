#!/usr/bin/env python3
"""Check the pinned npc-entrance viewer captures (requires Pillow)."""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageChops


def check(root: Path):
    states = [json.loads((root / f"entry-{i}.json").read_text()) for i in range(7)]
    assert len({s["handle"] for s in states}) == 1, "selected actor changed"
    assert len({tuple(s["camera"]) for s in states}) == 1, "camera changed"
    for state in states[:6]:
        assert state["state"] == "Resurrection"
        assert state["clip"] == "spawn"
        assert not state["allow_move"] and not state["allow_rotate"]
    assert states[6]["state"] != "Resurrection"
    assert states[6]["allow_move"] and states[6]["allow_rotate"]
    times = [states[i]["clip_time"] for i in (0, 1, 2, 5)]
    assert all(a < b for a, b in zip(times, times[1:])), "clip did not advance"
    for key in ("clip_time", "elapsed", "position", "roots"):
        assert states[2][key] == states[3][key] == states[4][key], key
    assert [s["hidden"] for s in states] == [False, False, False, True, False, False, False]

    images = [Image.open(root / f"{i:02}.png").convert("RGB") for i in range(1, 8)]
    visible, hidden, restored = images[2:5]
    assert ImageChops.difference(visible, restored).getbbox() is None, "restored pose changed"
    diff = ImageChops.difference(visible, hidden)
    mask = diff.convert("RGB").point(lambda x: 255 if x else 0).convert("L")
    count = sum(n for value, n in enumerate(mask.histogram()) if value)
    assert count > 0, "hiding the selected body changed no pixels"
    for a, b in ((1, 2), (2, 5), (5, 6)):
        assert ImageChops.difference(images[a], images[b]).getbbox(), "static pose capture"
    return {
        "handle": states[0]["handle"],
        "clip_times": times,
        "body_changed_pixels": count,
        "body_changed_bounds": diff.getbbox(),
        "restored_image_exact": True,
        "final_state": states[6]["state"],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("captures", type=Path)
    args = parser.parse_args()
    print(json.dumps(check(args.captures), indent=2))
