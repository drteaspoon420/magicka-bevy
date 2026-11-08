#!/usr/bin/env python3
"""Check original Earth weapon particle sprites in a pinned real-input capture.

The control suppresses only sprites 161, 7 and 197 from the submitted frame;
simulation state and the retained particles are untouched. Native parity is
unverified. This proves a rendered contribution, not its original appearance.
"""
import json
import sys
from pathlib import Path

from PIL import Image

run = Path(sys.argv[1])
rows = json.loads((run / "log.json").read_text())
images = {
    row["capture"]: Image.open(run / f"{i + 1:02}.png").convert("RGB")
    for i, row in enumerate(rows)
}
visible = images["trail_visible"]
hidden = images["trail_hidden"]
restored = images["trail_restored"]
changed = sum(
    max(abs(a - b) for a, b in zip(v, h)) > 12
    for v, h in zip(visible.get_flattened_data(), hidden.get_flattened_data())
)
restoration = [
    max(abs(a - b) for a, b in zip(v, r))
    for v, r in zip(visible.get_flattened_data(), restored.get_flattened_data())
]
# Use the same significance threshold for removal and restoration. Shader
# presentation can change by a few byte levels while gameplay time is pinned.
restored_changed = sum(error > 12 for error in restoration)
result = {
    "changed_pixels": changed,
    "restored_changed_pixels": restored_changed,
    "restored_max_channel_error": max(restoration),
    "native_visual_parity": "unverified",
    "passed": changed > 150 and restored_changed == 0,
}
(run / "earth-trail-pixels.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
if not result["passed"]:
    raise SystemExit("Earth weapon trail rendering regression")
