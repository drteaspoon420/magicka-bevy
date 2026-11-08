#!/usr/bin/env python3
"""Independent glTF skinning evaluator vs. game-math reference samples.

    python3 tools/check_skin.py <character.glb> [render.png] [--cull]

Reads `<character>.ref.json` (written by `character-gltf --reference`, which
computes skinned positions with the decompiled XNAnimation math directly from
the source data) and evaluates the .glb with standard glTF semantics: LINEAR
samplers (translation/scale lerp; rotation slerp per spec, and also nlerp to
isolate interpolation differences), node hierarchy, joint world matrices ·
inverse bind matrices, linear blend skinning. Prints the max position error
relative to the model's size. Optionally renders a side view (X/Y) of the
first sample. Requires numpy.
"""
import json
import struct
import sys
import zlib

import numpy as np


def load(path):
    data = open(path, "rb").read()
    jlen = struct.unpack_from("<I", data, 12)[0]
    gltf = json.loads(data[20:20 + jlen])
    off = 20 + jlen
    return gltf, data[off + 8:off + 8 + struct.unpack_from("<I", data, off)[0]]


def accessor(g, bin_, i):
    a = g["accessors"][i]
    v = g["bufferViews"][a["bufferView"]]
    n = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}[a["type"]]
    dt = {5126: np.float32, 5125: np.uint32, 5121: np.uint8, 5123: np.uint16}[a["componentType"]]
    arr = np.frombuffer(bin_, dtype=dt, count=a["count"] * n, offset=v.get("byteOffset", 0) + a.get("byteOffset", 0))
    return arr.reshape(a["count"], n) if n > 1 else arr


def quat_matrix(q):
    x, y, z, w = q
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                     [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                     [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])


def slerp(a, b, u):
    d = float(np.dot(a, b))
    if d < 0:
        b, d = -b, -d
    if d > 0.9995:
        r = a + u * (b - a)
        return r / np.linalg.norm(r)
    th = np.arccos(d)
    return (np.sin((1 - u) * th) * a + np.sin(u * th) * b) / np.sin(th)


def nlerp(a, b, u):
    if np.dot(a, b) < 0:
        b = -b
    r = (1 - u) * a + u * b
    return r / np.linalg.norm(r)


def sample(times, values, t, path, qinterp):
    if t <= times[0]:
        return values[0]
    if t >= times[-1]:
        return values[-1]
    i = int(np.searchsorted(times, t, side="right") - 1)
    u = (t - times[i]) / (times[i + 1] - times[i])
    if path == "rotation":
        return qinterp(values[i].astype(np.float64), values[i + 1].astype(np.float64), u)
    return values[i] + u * (values[i + 1] - values[i])


def evaluate(g, bin_, anim, t, qinterp):
    nodes = g["nodes"]
    trs = [{"translation": np.array(n.get("translation", [0, 0, 0]), float),
            "rotation": np.array(n.get("rotation", [0, 0, 0, 1]), float),
            "scale": np.array(n.get("scale", [1, 1, 1]), float)} for n in nodes]
    if anim is not None:
        a = g["animations"][anim]
        for ch in a["channels"]:
            s = a["samplers"][ch["sampler"]]
            times = accessor(g, bin_, s["input"]).astype(np.float64)
            vals = accessor(g, bin_, s["output"]).astype(np.float64)
            trs[ch["target"]["node"]][ch["target"]["path"]] = sample(times, vals, t, ch["target"]["path"], qinterp)
    world = [None] * len(nodes)

    def visit(i, parent):
        m = np.eye(4)
        m[:3, :3] = quat_matrix(trs[i]["rotation"]) * trs[i]["scale"]
        m[:3, 3] = trs[i]["translation"]
        world[i] = parent @ m
        for c in nodes[i].get("children", []):
            visit(c, world[i])

    for r in g["scenes"][0]["nodes"]:
        visit(r, np.eye(4))
    skinned = next(n for n in nodes if "skin" in n)
    skin = g["skins"][skinned["skin"]]
    ibm = accessor(g, bin_, skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1)
    jm = np.array([world[j] @ ibm[k] for k, j in enumerate(skin["joints"])])
    prim = g["meshes"][skinned["mesh"]]["primitives"][0]
    pos = accessor(g, bin_, prim["attributes"]["POSITION"]).astype(np.float64)
    jo = accessor(g, bin_, prim["attributes"]["JOINTS_0"]).astype(np.int64)
    we = accessor(g, bin_, prim["attributes"]["WEIGHTS_0"]).astype(np.float64)
    ph = np.c_[pos, np.ones(len(pos))]
    out = np.zeros((len(pos), 3))
    for k in range(4):
        out += we[:, k:k + 1] * np.einsum("nij,nj->ni", jm[jo[:, k]], ph)[:, :3]
    return out, g, bin_, prim


def render_side(points, tris, path, size=512, cull=False):
    lo, hi = points.min(0), points.max(0)
    sc = (size - 1) / max(hi[0] - lo[0], hi[1] - lo[1], 1e-6)
    w, h = int((hi[0] - lo[0]) * sc) + 1, int((hi[1] - lo[1]) * sc) + 1
    # Exact framing for `magicka-viewer --side --origin X Y --scale S --size W H`.
    print(f"FRAME {float(lo[0])!r} {float(hi[1])!r} {float(sc)!r} {w} {h}")
    img = np.zeros((h, w, 3), np.uint8)
    depth = np.full((h, w), -np.inf)
    for t in tris:
        p = points[t]
        n = np.cross(p[1] - p[0], p[2] - p[0])
        ln = np.linalg.norm(n)
        if ln < 1e-12:
            continue
        if cull and n[2] <= 0:
            continue  # back face toward the camera: a culling engine skips it
        k = 0.3 + 0.7 * abs(n[2] / ln)
        col = np.array([230, 200, 160]) * k if n[2] > 0 else np.array([90, 40, 40])
        sx, sy = (p[:, 0] - lo[0]) * sc, (hi[1] - p[:, 1]) * sc
        x0, x1 = max(int(sx.min()), 0), min(int(np.ceil(sx.max())), w - 1)
        y0, y1 = max(int(sy.min()), 0), min(int(np.ceil(sy.max())), h - 1)
        if x1 < x0 or y1 < y0:
            continue
        xs, ys = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(y0, y1 + 1) + 0.5)
        d = (sx[1] - sx[0]) * (sy[2] - sy[0]) - (sx[2] - sx[0]) * (sy[1] - sy[0])
        if abs(d) < 1e-12:
            continue
        w0 = ((sx[1] - xs) * (sy[2] - ys) - (sx[2] - xs) * (sy[1] - ys)) / d
        w1 = ((sx[2] - xs) * (sy[0] - ys) - (sx[0] - xs) * (sy[2] - ys)) / d
        w2 = 1 - w0 - w1
        z = w0 * p[0, 2] + w1 * p[1, 2] + w2 * p[2, 2]
        sub = depth[y0:y1 + 1, x0:x1 + 1]
        m = (w0 >= 0) & (w1 >= 0) & (w2 >= 0) & (z > sub)
        sub[m] = z[m]
        img[y0:y1 + 1, x0:x1 + 1][m] = col.astype(np.uint8)
    raw = b"".join(b"\x00" + img[y].tobytes() for y in range(h))

    def chunk(k, b):
        return struct.pack(">I", len(b)) + k + b + struct.pack(">I", zlib.crc32(k + b))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def main():
    cull = "--cull" in sys.argv
    sys.argv = [a for a in sys.argv if a != "--cull"]
    glb = sys.argv[1]
    ref = json.load(open(glb[:-4] + ".ref.json"))
    g, bin_ = load(glb)
    n = ref["vertices"]
    worst = {"slerp": 0.0, "nlerp": 0.0}
    size = 0.0
    for s in ref["samples"]:
        expect = np.array(s["positions"], float)
        size = max(size, float(np.linalg.norm(expect.max(0) - expect.min(0))))
        for name, fn in (("slerp", slerp), ("nlerp", nlerp)):
            got, *_ = evaluate(g, bin_, s["animation"], s["time"], fn)
            worst[name] = max(worst[name], float(np.abs(got[:n] - expect).max()))
    print(json.dumps({"samples": len(ref["samples"]), "vertices": n, "model_size": round(size, 4),
                      "max_error_slerp": worst["slerp"], "max_error_nlerp": worst["nlerp"],
                      "relative_slerp": worst["slerp"] / max(size, 1e-9), "relative_nlerp": worst["nlerp"] / max(size, 1e-9)}))
    if len(sys.argv) > 2 and ref["samples"]:
        s = ref["samples"][len(ref["samples"]) // 2]
        pts, g2, b2, prim = evaluate(g, bin_, s["animation"], s["time"], nlerp)
        tris = accessor(g2, b2, prim["indices"]).astype(np.int64).reshape(-1, 3)
        render_side(pts, tris, sys.argv[2], cull=cull)
        print("rendered", g["animations"][s["animation"]]["name"], "t =", round(s["time"], 3))
        print(f"SAMPLE {s['animation']} {s['time']!r}")


if __name__ == "__main__":
    main()
