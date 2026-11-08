#!/usr/bin/env python3
"""GPU parity check of the Bevy terrain material (terrain.wgsl) against the
bytecode-verified closed-form blend (output/evidence/tools/check_terrain_formula.py).

    python3 tools/terrain_check.py <scratch-dir>

Renders `magicka-viewer --terrain-test` for several flag/sharpness
combinations and compares every pixel with the expected value computed from
the same test definitions (see `terrain_test` in the viewer). Prints max
8-bit difference and discard mismatches per case; exit 1 on failure.
"""
import os
import subprocess
import sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "../../evidence/tools"))
from check_terrain_formula import closed_form  # noqa: E402

VIEWER = os.path.join(HERE, "../target/release/magicka-viewer")
TEX0, A0_OFF, MASK0, A1_OFF, MASK1, DUAL, VCOLOR, UNLIT = 1, 2, 4, 8, 16, 32, 64, 128
CASES = {
    "mask0 blend (common terrain)": TEX0 | MASK0 | DUAL | VCOLOR | UNLIT,
    "vertex-alpha blend + cutout": TEX0 | DUAL | VCOLOR | UNLIT,
    "mask0+mask1": TEX0 | MASK0 | MASK1 | DUAL | VCOLOR | UNLIT,
    "mask1 only": TEX0 | MASK1 | DUAL | VCOLOR | UNLIT,
    "both alphas disabled": TEX0 | A0_OFF | A1_OFF | DUAL | UNLIT,
    "single layer cutout": TEX0 | UNLIT,
    "single layer mask0 (no cut)": TEX0 | MASK0 | VCOLOR | UNLIT,
}


def texture(seed, size=64):
    i = np.arange(size * size * 4, dtype=np.uint64)
    x = ((i + np.uint64((seed * 7919) & 0xFFFFFFFF)) & np.uint64(0xFFFFFFFF)) * np.uint64(2654435761)
    x &= np.uint64(0xFFFFFFFF)
    return (((x >> np.uint64(13)) & np.uint64(0xFF)).astype(np.float64) / 255.0).reshape(size, size, 4)


def expected(flags, sharp):
    t0, t1 = texture(1), texture(2)
    col0, col1 = (1.0, 0.9, 0.8), (0.7, 1.0, 1.2)
    out = np.zeros((256, 256, 3))
    killed = np.zeros((256, 256), bool)
    for y in range(256):
        wy = 256 - (y + 0.5)
        for x in range(256):
            px, py = (x + 0.5) / 256, wy / 256
            u0 = (2 * px, 2 * py)
            u1 = (3 * py, 3 * px)
            s0 = t0[int((u0[1] % 1) * 64), int((u0[0] % 1) * 64)]
            s1 = t1[int((u1[1] % 1) * 64), int((u1[0] % 1) * 64)]
            vcol = (px, py, 0.5, 0.6 * px + 0.4 * py) if flags & VCOLOR else (1, 1, 1, 1)
            if flags & DUAL:
                alb, k = closed_form(list(s0), list(s1), col0, col1, vcol, sharp, bool(flags & TEX0),
                                     bool(flags & A0_OFF), bool(flags & MASK0), bool(flags & A1_OFF), bool(flags & MASK1))
            else:
                c0 = [s0[i] * col0[i] for i in range(3)] if flags & TEX0 else list(col0)
                a0 = 1.0 if (flags & A0_OFF or not flags & TEX0) else s0[3]
                cov = 1.0 if flags & MASK0 else a0
                alb, k = [c0[i] * vcol[i] for i in range(3)], cov - 0.5 < 0
            out[y, x] = alb
            killed[y, x] = k
    return np.clip(np.round(out * 255), 0, 255), killed


def main():
    scratch = sys.argv[1]
    ok = True
    for name, flags in CASES.items():
        for sharp in (0.0, 1.0, 3.0):
            path = os.path.join(scratch, f"terrain_{flags}_{sharp}.png")
            r = subprocess.run([VIEWER, "--terrain-test", path, "--tflags", str(flags), "--sharp", str(sharp)],
                               capture_output=True, text=True, timeout=300)
            if r.returncode != 0 or not os.path.exists(path):
                print(f"{name} s={sharp}: render failed: {r.stderr[-300:]}")
                ok = False
                continue
            got = np.asarray(Image.open(path).convert("RGB")).astype(int)
            exp, killed = expected(flags, sharp)
            magenta = (got[..., 0] == 255) & (got[..., 1] == 0) & (got[..., 2] == 255)
            kill_mismatch = int((magenta != killed).sum())
            live = ~killed & ~magenta
            diff = np.abs(got - exp)[live]
            maxd = int(diff.max()) if diff.size else 0
            over = int((diff.max(axis=1) > 1).sum()) if diff.size else 0
            print(f"{name:32} s={sharp}: max diff {maxd}/255, pixels >1: {over}, discard mismatches {kill_mismatch}, kept {int(live.sum())}")
            ok &= maxd <= 2 and kill_mismatch <= 64 * 0  # strict: discards must match exactly
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
