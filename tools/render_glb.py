#!/usr/bin/env python3
"""Independent top-down render of a .glb (decoder separate from the Rust writer).

    python3 tools/render_glb.py scene.glb out.png [size] [--cull]

Walks the node hierarchy (matrix or TRS rest pose), transforms every
primitive to world space, and rasterizes X/Z with a height buffer. Colors:
static = grey, animated_parts = orange, liquids = blue, physics_props = green,
shaded by facing. Back-facing triangles (seen from above, after CCW winding)
are drawn darker to make winding problems visible, or skipped with --cull
(matches a back-face-culling engine such as Bevy for comparisons). Prints per-group triangle
counts and world bounds. Requires numpy.
"""
import json
import struct
import sys
import zlib

import numpy as np

GROUP_COLORS = {"static": (0.75, 0.75, 0.75), "animated_parts": (1.0, 0.55, 0.1),
                "liquids": (0.2, 0.45, 1.0), "physics_props": (0.2, 0.9, 0.3)}


def load(path):
    data = open(path, "rb").read()
    assert data[:4] == b"glTF"
    jlen = struct.unpack_from("<I", data, 12)[0]
    gltf = json.loads(data[20:20 + jlen])
    off = 20 + jlen
    bin_ = data[off + 8:off + 8 + struct.unpack_from("<I", data, off)[0]] if off < len(data) else b""
    return gltf, bin_


def accessor(gltf, bin_, i):
    a = gltf["accessors"][i]
    v = gltf["bufferViews"][a["bufferView"]]
    n = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
    dt = {5126: np.float32, 5125: np.uint32}[a["componentType"]]
    start = v.get("byteOffset", 0) + a.get("byteOffset", 0)
    arr = np.frombuffer(bin_, dtype=dt, count=a["count"] * n, offset=start)
    return arr.reshape(a["count"], n) if n > 1 else arr


def local_matrix(node):
    # Column-vector 4x4 matrix.
    if "matrix" in node:
        return np.array(node["matrix"], dtype=np.float64).reshape(4, 4).T
    t = node.get("translation", [0, 0, 0])
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    s = node.get("scale", [1, 1, 1])
    r = np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                  [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                  [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])
    m = np.eye(4)
    m[:3, :3] = r * np.array(s)
    m[:3, 3] = t
    return m


def main():
    cull = "--cull" in sys.argv
    argv = [a for a in sys.argv if a != "--cull"]
    src, dst = argv[1], argv[2]
    size = int(argv[3]) if len(argv) > 3 else 1024
    gltf, bin_ = load(src)
    nodes = gltf["nodes"]
    tris = []  # (group, world vertices Nx3x3)

    def visit(i, parent, group):
        n = nodes[i]
        g = n.get("name", "") if n.get("name", "") in GROUP_COLORS else group
        world = parent @ local_matrix(n)
        if "mesh" in n:
            for p in gltf["meshes"][n["mesh"]]["primitives"]:
                pos = accessor(gltf, bin_, p["attributes"]["POSITION"]).astype(np.float64)
                idx = accessor(gltf, bin_, p["indices"]).astype(np.int64)
                w = (np.c_[pos, np.ones(len(pos))] @ world.T)[:, :3]
                tris.append((g, w[idx.reshape(-1, 3)]))
        for c in n.get("children", []):
            visit(c, world, g)

    for r in gltf["scenes"][0]["nodes"]:
        visit(r, np.eye(4), "static")
    allv = np.concatenate([t.reshape(-1, 3) for _, t in tris])
    lo, hi = allv.min(0), allv.max(0)
    scale = (size - 1) / max(hi[0] - lo[0], hi[2] - lo[2])
    w, h = int((hi[0] - lo[0]) * scale) + 1, int((hi[2] - lo[2]) * scale) + 1
    img = np.zeros((h, w, 3), np.uint8)
    depth = np.full((h, w), -np.inf)
    counts = {}
    order = ["static", "liquids", "physics_props", "animated_parts"]
    for g, t in sorted(tris, key=lambda x: order.index(x[0]) if x[0] in order else 0):
        counts[g] = counts.get(g, 0) + len(t)
        base = np.array(GROUP_COLORS.get(g, (1, 0, 1)))
        for tri in t:
            n = np.cross(tri[1] - tri[0], tri[2] - tri[0])
            ln = np.linalg.norm(n)
            if ln < 1e-12:
                continue
            facing = n[1] / ln  # +Y = up-facing (CCW seen from above)
            if cull and facing < 0:
                continue  # back face from above: a culling engine skips it
            k = 0.35 + 0.65 * abs(facing) if facing >= 0 else 0.15
            col = (base * k * 255).astype(np.uint8)
            sx = (tri[:, 0] - lo[0]) * scale
            sy = (tri[:, 2] - lo[2]) * scale
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
            inside = (w0 >= 0) & (w1 >= 0) & (w2 >= 0)
            z = w0 * tri[0, 1] + w1 * tri[1, 1] + w2 * tri[2, 1]
            sub = depth[y0:y1 + 1, x0:x1 + 1]
            mask = inside & (z > sub)
            sub[mask] = z[mask]
            img[y0:y1 + 1, x0:x1 + 1][mask] = col
    raw = b"".join(b"\x00" + img[y].tobytes() for y in range(h))
    def chunk(k, b):
        return struct.pack(">I", len(b)) + k + b + struct.pack(">I", zlib.crc32(k + b))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
    open(dst, "wb").write(png)
    print("triangles per group:", counts)
    # Exact framing for `magicka-viewer --top --origin X Z --scale S --size W H`.
    print(f"FRAME {float(lo[0])!r} {float(lo[2])!r} {float(scale)!r} {w} {h}")
    print("world bounds:", np.round(lo, 2).tolist(), np.round(hi, 2).tolist())


if __name__ == "__main__":
    main()
