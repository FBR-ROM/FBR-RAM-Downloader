#!/usr/bin/env python3
"""Regenerate src/model_data.rs from the model sources in models/.

Three models are embedded, in rotation order:
  1. FBR nano logo  (models/fbrnanologo.obj)  — used as-is (already small)
  2. v100           (models/v100.obj)         — welded, decimated, normalized
  3. P5000          (models/P5000.stl)        — welded, decimated, normalized

The big models are decimated to ~2000 triangles (keeping the largest-area
triangles, which preserves the silhouette) and normalized so their largest
extent is 1.0 and they're centered on the origin — matching the scale of the
FBR logo so all three fill the same space on screen.

Usage:  python3 gen_models.py
"""
import math
import os
import struct

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "src", "model_data.rs")
TARGET_TRIS = 2000


# --- loaders -----------------------------------------------------------------

def load_obj(path):
    """Return (verts, faces) from an OBJ file. Faces are triangulated (fan)."""
    verts, faces = [], []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line.startswith("v "):
                p = line.split()
                verts.append((float(p[1]), float(p[2]), float(p[3])))
            elif line.startswith("f "):
                idx = [int(t.split("/")[0]) - 1 for t in line.split()[1:]]
                for i in range(1, len(idx) - 1):
                    faces.append((idx[0], idx[i], idx[i + 1]))
    return verts, faces


def load_stl(path):
    """Return (verts, faces) from a binary STL file (one vert per corner)."""
    with open(path, "rb") as f:
        data = f.read()
    ntri = struct.unpack_from("<I", data, 80)[0]
    verts, faces = [], []
    for i in range(ntri):
        off = 84 + i * 50
        vals = struct.unpack_from("<12fH", data, off)
        base = len(verts)
        for v in range(3):
            verts.append((vals[3 + v * 3], vals[4 + v * 3], vals[5 + v * 3]))
        faces.append((base, base + 1, base + 2))
    return verts, faces


# --- mesh utilities ----------------------------------------------------------

def weld(verts):
    """Merge vertices that round to the same point. Returns (welded, map)."""
    weld_map, welded = {}, []
    for v in verts:
        k = (round(v[0], 4), round(v[1], 4), round(v[2], 4))
        if k not in weld_map:
            weld_map[k] = len(welded)
            welded.append(v)
    return welded, weld_map


def tri_area(a, b, c):
    ax, ay, az = a
    bx, by, bz = b
    cx, cy, cz = c
    ux, uy, uz = bx - ax, by - ay, bz - az
    vx, vy, vz = cx - ax, cy - ay, cz - az
    nx, ny, nz = uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx
    return 0.5 * math.sqrt(nx * nx + ny * ny + nz * nz)


def decimate(verts, faces, target):
    """Keep the `target` largest-area triangles; reindex to a compact mesh."""
    areas = [tri_area(verts[a], verts[b], verts[c]) for a, b, c in faces]
    order = sorted(range(len(faces)), key=lambda i: areas[i], reverse=True)
    keep = set(order[:target])
    used = set()
    for i in keep:
        used.update(faces[i])
    remap = {v: i for i, v in enumerate(sorted(used))}
    nv = [verts[v] for v in sorted(used)]
    nf = [(remap[a], remap[b], remap[c]) for i in keep for a, b, c in [faces[i]]]
    return nv, nf


def normalize(verts):
    """Scale so the largest extent is 1.0 and center on the centroid."""
    xs = [v[0] for v in verts]
    ys = [v[1] for v in verts]
    zs = [v[2] for v in verts]
    cx = (min(xs) + max(xs)) / 2
    cy = (min(ys) + max(ys)) / 2
    cz = (min(zs) + max(zs)) / 2
    ext = max(max(xs) - min(xs), max(ys) - min(ys), max(zs) - min(zs))
    s = 1.0 / ext
    return [((x - cx) * s, (y - cy) * s, (z - cz) * s) for x, y, z in verts]


# --- emit --------------------------------------------------------------------

def emit(f, name, verts, faces, comment):
    f.write(f"// Auto-generated from {comment}. Do not edit by hand.\n")
    f.write(f"// {len(verts)} vertices, {len(faces)} triangles (0-based indices).\n\n")
    f.write(f"pub const VERTS_{name}: &[(f32, f32, f32)] = &[\n")
    for x, y, z in verts:
        f.write(f"    ({x:.5f}, {y:.5f}, {z:.5f}),\n")
    f.write("];\n\n")
    f.write(f"pub const FACES_{name}: &[(usize, usize, usize)] = &[\n")
    for a, b, c in faces:
        f.write(f"    ({a}, {b}, {c}),\n")
    f.write("];\n\n")


def main():
    with open(OUT, "w") as out:
        out.write("// Embedded 3D models for the cracktro. Regenerate with gen_models.py.\n\n")
        # FBR nano logo (bare VERTS/FACES names), used as-is.
        v, f = load_obj(os.path.join(HERE, "models", "fbrnanologo.obj"))
        print(f"FBR nano: {len(v)} verts, {len(f)} tris")
        out.write("// Auto-generated from fbrnanologo.obj. Do not edit by hand.\n")
        out.write(f"// {len(v)} vertices, {len(f)} triangles (0-based indices).\n\n")
        out.write("pub const VERTS: &[(f32, f32, f32)] = &[\n")
        for x, y, z in v:
            out.write(f"    ({x:.5f}, {y:.5f}, {z:.5f}),\n")
        out.write("];\n\n")
        out.write("pub const FACES: &[(usize, usize, usize)] = &[\n")
        for a, b, c in f:
            out.write(f"    ({a}, {b}, {c}),\n")
        out.write("];\n\n")
        # v100 — weld, decimate, normalize.
        v, f = load_obj(os.path.join(HERE, "models", "v100.obj"))
        welded, wmap = weld(v)
        f = [tuple(wmap[(round(v[i][0], 4), round(v[i][1], 4), round(v[i][2], 4))] for i in tri) for tri in f]
        f = [tri for tri in f if len(set(tri)) == 3]
        n_before = len(f)
        v, f = decimate(welded, f, TARGET_TRIS)
        v = normalize(v)
        print(f"v100: {len(v)} verts, {len(f)} tris (decimated from {n_before})")
        emit(out, "V100", v, f, "v100.obj (decimated + normalized)")
        # P5000 — weld, decimate, normalize.
        v, f = load_stl(os.path.join(HERE, "models", "P5000.stl"))
        welded, wmap = weld(v)
        f = [tuple(wmap[(round(v[i][0], 4), round(v[i][1], 4), round(v[i][2], 4))] for i in tri) for tri in f]
        f = [tri for tri in f if len(set(tri)) == 3]
        n_before = len(f)
        v, f = decimate(welded, f, TARGET_TRIS)
        v = normalize(v)
        print(f"P5000: {len(v)} verts, {len(f)} tris (decimated from {n_before})")
        emit(out, "P5000", v, f, "P5000.stl (decimated + normalized)")

    print(f"wrote {OUT} ({os.path.getsize(OUT)} bytes)")


if __name__ == "__main__":
    main()
