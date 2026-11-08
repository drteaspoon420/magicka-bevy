#!/usr/bin/env python3
"""GPU regression for particles versus opaque meshes excluded from the prepass.

Usage: python3 output/rewrite/tools/check_particle_depth.py VIEWER OUTPUT_DIR
Run under xvfb-run when the host has no usable display. Uses procedural assets.
"""
import json
import math
import os
from pathlib import Path
import subprocess
import sys

from PIL import Image


def main():
    viewer = str(Path(sys.argv[1]).resolve())
    out = Path(sys.argv[2]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    reports = []
    for samples in (1, 4):
        for alpha in (0, 1):
            shots = {}
            for name, z, full_depth in (
                ("front", 2.0, True),
                ("intersect", 0.5, True),
                ("behind", -1.0, True),
                ("old-prepass", -1.0, False),
            ):
                stem = f"{samples}x-alpha{alpha}-{name}"
                scene = {
                    "eye": [0, 0, 10], "target": [0, 0, 0], "up": [0, 1, 0],
                    "fov": math.pi / 4, "near": 0.1, "far": 100, "size": 256,
                    "msaa": samples, "full_depth": full_depth,
                    "occluders": [{"position": [0, 0, 0], "size": [4, 4, 1]}],
                    "particles": [{
                        "position": [0, 0, z], "radius": 0.5, "rotation": 0,
                        "life": 0.5, "color": [0, 0, 1, 1], "sprite": 0,
                        "alpha_blended": alpha, "hsv_colorize": 0,
                        "rotation_aligned": 0, "velocity": [0, 0, 0],
                    }],
                }
                source, capture = out / f"{stem}.json", out / f"{stem}.png"
                source.write_text(json.dumps(scene, indent=2) + "\n")
                env = dict(os.environ)
                env.pop("WAYLAND_DISPLAY", None)
                env.pop("WAYLAND_SOCKET", None)
                result = subprocess.run(
                    [viewer, "--particle-test", str(source), str(capture)],
                    env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                    timeout=120,
                )
                (out / f"{stem}.log").write_text(result.stdout)
                if (result.returncode or "read-only file system" in result.stdout.lower()
                        or "Validation Error" in result.stdout or not capture.exists()):
                    raise RuntimeError(result.stdout[-6000:])
                shots[name] = Image.open(capture).convert("RGB")
            points = [(118, 118), (138, 118), (118, 138), (138, 138)]
            front = [shots["front"].getpixel(p)[2] for p in points]
            middle = [shots["intersect"].getpixel(p)[2] for p in points]
            rear_max = max(high for _, high in shots["behind"].getextrema())
            old_max = max(high for _, high in shots["old-prepass"].getextrema())
            # ParticleEffect's alpha is multiplied by soft=0.5 at the surface,
            # then its RGB uses 1-exp2(-rgb*alpha), rather than a linear fade.
            error = max(abs(m - 255 * (1 - math.sqrt(1 - f / 255)))
                        for f, m in zip(front, middle))
            assert min(front) > 5, (samples, alpha, "front invisible", front)
            assert rear_max == 0, (samples, alpha, "behind leaks", rear_max)
            assert old_max > 5, (samples, alpha, "control did not reproduce bug")
            assert error <= 3, (samples, alpha, "soft intersection", error, front, middle)
            reports.append({"msaa": samples, "alpha_blended": alpha,
                            "behind_max": rear_max, "old_prepass_max": old_max,
                            "soft_fade_error_255": error})
    report = {"status": "PASS", "cases": reports}
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
