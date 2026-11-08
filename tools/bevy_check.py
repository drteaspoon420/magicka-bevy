#!/usr/bin/env python3
"""Batch check: Bevy GPU renders vs. independent reference renders.

    python3 tools/bevy_check.py <cache-dir> [--levels] [--characters] [--jobs N] [--limit N]

For each level .glb: top-down reference (`render_glb.py`) vs `magicka-viewer
--top --opaque` with identical framing. For each character .glb: the middle
reference skinning sample (`check_skin.py`, game-math interpolation) vs
`magicka-viewer --side --anim A --time T --opaque`. Prints per-file IoU of
silhouettes, a summary, and writes `<cache>/check/bevy_check.tsv`.
Requires the viewer built in release mode and a GPU.
"""
import concurrent.futures as cf
import json
import os
import subprocess
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
VIEWER = os.path.join(TOOLS, "..", "target", "release", "magicka-viewer")


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=600)


def check(cache, kind, rel):
    try:
        return check_inner(cache, kind, rel)
    except Exception as e:  # record, don't abort the batch
        return rel, {"error": f"{type(e).__name__}: {e}"}


def check_inner(cache, kind, rel):
    stem = rel.replace("/", "_")[:-4]
    ref = os.path.join(cache, "check", f"{stem}.ref.png")
    bevy = os.path.join(cache, "check", f"{stem}.bevy.png")
    if kind == "level":
        out = run(["python3", os.path.join(TOOLS, "render_glb.py"), os.path.join(cache, rel), ref, "384", "--cull"]).stdout
        frame = next(l for l in out.splitlines() if l.startswith("FRAME")).split()
        extra = ["--top"]
    else:
        out = run(["python3", os.path.join(TOOLS, "check_skin.py"), os.path.join(cache, rel), ref, "--cull"]).stdout
        lines = out.splitlines()
        parity = json.loads(lines[0])
        frame = next(l for l in lines if l.startswith("FRAME")).split()
        sample = next(l for l in lines if l.startswith("SAMPLE")).split()
        extra = ["--side", "--anim", sample[1], "--time", sample[2]]
    r = run([VIEWER, "--root", cache, "--scene", rel, "--render", bevy, "--origin", frame[1], frame[2],
             "--scale", frame[3], "--size", frame[4], frame[5], "--opaque", *extra])
    if r.returncode != 0 or not os.path.exists(bevy):
        return rel, {"error": (r.stderr or r.stdout)[-300:]}
    cmp_ = json.loads(run(["python3", os.path.join(TOOLS, "compare_silhouette.py"), bevy, ref]).stdout)
    if kind == "character":
        cmp_["parity_relative_nlerp"] = parity["relative_nlerp"]
    return rel, cmp_


def main():
    cache = os.path.abspath(sys.argv[1])
    args = sys.argv[2:]
    jobs = int(args[args.index("--jobs") + 1]) if "--jobs" in args else 4
    limit = int(args[args.index("--limit") + 1]) if "--limit" in args else None
    os.makedirs(os.path.join(cache, "check"), exist_ok=True)
    work = []
    for kind, sub, flag in (("level", "gltf/Levels", "--levels"), ("character", "gltf/Data/Characters", "--characters")):
        if flag not in args:
            continue
        files = []
        for root, _, names in os.walk(os.path.join(cache, sub)):
            files += [os.path.relpath(os.path.join(root, n), cache) for n in names if n.endswith(".glb")]
        files.sort()
        work += [(kind, f) for f in files[:limit]]
    results = {}
    with cf.ThreadPoolExecutor(jobs) as ex:
        for rel, res in ex.map(lambda w: check(cache, *w), work):
            results[rel] = res
            print(rel, json.dumps(res), flush=True)
    with open(os.path.join(cache, "check", "bevy_check.tsv"), "w") as f:
        for rel, res in sorted(results.items()):
            f.write(f"{rel}\t{json.dumps(res)}\n")
    ious = sorted((r["iou"], k) for k, r in results.items() if "iou" in r)
    errors = [k for k, r in results.items() if "error" in r]
    if ious:
        vals = [v for v, _ in ious]
        print(f"\nchecked {len(ious)} files, errors {len(errors)}; IoU min {vals[0]:.4f} median {vals[len(vals) // 2]:.4f}")
        print("lowest:", [(f"{v:.4f}", k) for v, k in ious[:8]])
    for e in errors[:10]:
        print("ERROR", e, results[e]["error"])


if __name__ == "__main__":
    main()
