#!/usr/bin/env python3
"""Compare a Bevy render (magenta background) with a reference render (black
background) by silhouette: IoU of covered pixels.

    python3 tools/compare_silhouette.py bevy.png reference.png [diff.png]

Prints JSON with IoU and pixel counts. The optional diff image shows
both = grey, Bevy only = green, reference only = red.
"""
import json
import sys

import numpy as np
from PIL import Image


def main():
    bevy = np.asarray(Image.open(sys.argv[1]).convert("RGB")).astype(int)
    ref = np.asarray(Image.open(sys.argv[2]).convert("RGB")).astype(int)
    if bevy.shape != ref.shape:
        print(json.dumps({"error": f"size mismatch {bevy.shape} vs {ref.shape}"}))
        sys.exit(1)
    magenta = (bevy[..., 0] > 200) & (bevy[..., 1] < 60) & (bevy[..., 2] > 200)
    b = ~magenta
    r = ref.sum(-1) > 0
    inter, union = int((b & r).sum()), int((b | r).sum())
    out = {"iou": inter / max(union, 1), "bevy_px": int(b.sum()), "ref_px": int(r.sum()),
           "bevy_only": int((b & ~r).sum()), "ref_only": int((r & ~b).sum())}
    print(json.dumps(out))
    if len(sys.argv) > 3:
        d = np.zeros(bevy.shape, np.uint8)
        d[b & r] = (128, 128, 128)
        d[b & ~r] = (0, 255, 0)
        d[r & ~b] = (255, 0, 0)
        Image.fromarray(d).save(sys.argv[3])


if __name__ == "__main__":
    main()
