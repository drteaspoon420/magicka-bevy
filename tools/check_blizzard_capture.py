#!/usr/bin/env python3
"""Check the real-input Blizzard capture's queue, lifetime and scroll state."""
import json
from pathlib import Path
import re
import sys


def check(run: Path) -> dict:
    rows = {row["capture"]: row for row in json.loads((run / "log.json").read_text())}
    log = (run / "viewer.log").read_text()
    if rows["queued"]["queue"] != [4, 256, 4]:
        raise ValueError("expected Cold/Ice/Cold before Space")
    states = []
    for name in ["blizzard", "blizzard_later"]:
        match = re.search(
            rf"autopilot {name}: blizzard=Some\(Blizzard .*?ttl: ([\d.eE+-]+), alpha: ([\d.eE+-]+), offsets: (.*?)\}}\)",
            log,
        )
        if match is None or float(match[1]) <= 0 or float(match[2]) != 1.0:
            raise ValueError(f"missing fully active Blizzard at {name}")
        states.append((float(match[1]), match[3]))
    if states[1][0] >= states[0][0] or states[0][1] == states[1][1]:
        raise ValueError("Blizzard TTL/scroll did not advance")
    if "autopilot ended: blizzard=None" not in log:
        raise ValueError("Blizzard did not expire")
    return {"scenario": "blizzard", "queue_and_lifetime": "pass",
            "active_ttl": [s[0] for s in states], "visual_parity": "unverified"}


if __name__ == "__main__":
    run = Path(sys.argv[1])
    report = check(run)
    (run / "blizzard-check.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
