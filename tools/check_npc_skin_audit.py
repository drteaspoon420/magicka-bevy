#!/usr/bin/env python3
"""Summarize the attack-autopilot's sampled CPU skin and visibility audits.

This checks logged CPU measurements, not GPU descriptors or retail parity.
The viewer itself validates all four joint indices, finite/nonnegative weights,
finite blended matrices and transformed positions for every sampled vertex.
"""
import argparse
import json
import math
import re
from pathlib import Path


def summarize(path):
    source = path.read_text()
    samples = re.findall(
        r"NPC weighted skin audit mesh=(\S+) vertices=(\d+) "
        r"min_abs_det=(\S+) max_weight_error=(\S+)", source
    )
    if not samples:
        raise ValueError(f"{path}: no weighted skin samples")
    if "panicked at" in source:
        raise ValueError(f"{path}: viewer assertion failed")
    meshes = {}
    for mesh, count, determinant, error in samples:
        determinant, error = float(determinant), float(error)
        if not all(math.isfinite(x) for x in (determinant, error)):
            raise ValueError(f"{path}: nonfinite audit measurement")
        item = meshes.setdefault(mesh, {
            "vertices": int(count), "samples": 0,
            "min_abs_det": determinant, "max_weight_error": error,
        })
        if item["vertices"] != int(count):
            raise ValueError(f"{path}: mesh vertex count changed")
        item["samples"] += 1
        item["min_abs_det"] = min(item["min_abs_det"], determinant)
        item["max_weight_error"] = max(item["max_weight_error"], error)
    visibility = {}
    for mesh, inherited, visible in re.findall(
        r"NPC visibility audit mesh=(\S+) inherited=(true|false) view=(true|false)", source
    ):
        item = visibility.setdefault(mesh, {"samples": 0, "inherited_visible": 0, "view_visible": 0})
        item["samples"] += 1
        item["inherited_visible"] += inherited == "true"
        item["view_visible"] += visible == "true"
    return {
        "log": str(path),
        "visibility_samples": visibility,
        "last_sample_frame": max(map(int, re.findall(r"NPC attack audit frame=(\d+)", source))),
        "meshes": meshes,
        "visibility_controls": re.findall(r"NPC visibility control (.*)", source),
        "segfault": "Segmentation fault" in source or "native fault signal=11" in source,
        "final_screenshot_saved": bool(re.search(r"Screenshot saved to .*final\.png", source)),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", type=Path)
    args = parser.parse_args()
    print(json.dumps([summarize(path) for path in args.logs], indent=2))


if __name__ == "__main__":
    main()
