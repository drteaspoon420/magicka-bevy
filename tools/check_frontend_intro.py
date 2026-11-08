#!/usr/bin/env python3
"""Audit private --intro-preview captures; never runs the original game."""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageStat


def audit(directory: Path, skipped: bool) -> dict:
    expected = {
        "dark": 0, "opening": 0, "chapter": 0, "page1": 1, "page2": 2,
        "pan_start": 3, "pan_end": 3, "zoom1_start": 4, "zoom1_end": 4,
        "zoom2_start": 5, "zoom2_end": 5, "page6": 6, "fade": 6, "handoff": 6,
    }
    hashes = {}
    for name, texture in expected.items():
        record = json.loads((directory / f"{name}.json").read_text())
        image = Image.open(directory / f"{name}.png").convert("RGB")
        assert image.size == (1024, 768), (name, image.size)
        hashes[name] = hashlib.sha256(image.tobytes()).hexdigest()
        if not skipped:
            assert record["texture"] == texture, (name, record)
            assert 0 <= record["motion"] <= 1, (name, record)
        if name == "opening" and not skipped:
            assert record["subtitles"], record
        if name == "handoff":
            assert record["finished"], record
            assert not record["subtitles"], "exit retained opening subtitles"
            assert max(ImageStat.Stat(image).mean) < 1, "fade did not reach black"
        if name == "dark":
            assert image.getbbox() is None, "dark page still draws visible content"
        if name == "opening":
            assert image.crop((128, 64, 896, 600)).getbbox() is None, "chapter text appeared before the light"

    events = json.loads((directory / "events.json").read_text())
    loads = [event for event in events if "LoadFirstLevel" in event["event"]]
    assert len(loads) == 1, loads
    fades = [event for event in events if "FadeToBlack" in event["event"]]
    assert len(fades) == 1, fades
    if skipped:
        assert "loading_tip: None" in loads[0]["event"], loads
        assert not any("PreparePage" in e["event"] for e in events), events
    else:
        assert "loading_tip: Some(7)" in loads[0]["event"], loads
        assert abs(fades[0]["time"] - 101) < 0.2, fades
        assert abs(loads[0]["time"] - 103) < 0.25, loads
        narrated = [e["event"] for e in events if "PlayNarration" in e["event"]]
        assert narrated == [f"PlayNarration({i})" for i in range(7)], narrated
        pages = ["chapter", "page1", "page2", "pan_start", "zoom1_start", "zoom2_start", "page6"]
        assert len({hashes[name] for name in pages}) == len(pages), "repeated page images"
        for start, end in [("pan_start", "pan_end"), ("zoom1_start", "zoom1_end"), ("zoom2_start", "zoom2_end")]:
            assert hashes[start] != hashes[end], f"no motion: {start} -> {end}"
    return {"captures": len(hashes), "skipped": skipped, "hashes": hashes, "events": len(events)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--skipped", action="store_true")
    args = parser.parse_args()
    print(json.dumps(audit(args.directory, args.skipped), indent=2))
